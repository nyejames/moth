//! Sparse allocation layer: which allocations hold which other allocations.
//!
//! WHAT: names allocations by an interned base plus a field path and records explicit holder
//!       edges for values that a construction, field write or collection insert stores by
//!       reference. A (holder, field) pair with no explicit edge holds its lazy child: the
//!       original contents of that field, named `holder.field`.
//! WHY: root-granular provenance names only bindings, so a value stored inside another value
//!      loses its identity once it is reached through a field. The layer keeps that identity
//!      for exactly the values that were stored by reference, and costs nothing for a function
//!      that never stores one.

use super::{
    AddressSet, BindingAlt, BindingRow, BindingSpec, BindingTarget, BorrowState, BoundaryBinding,
    CapturedRoutes, CapturedValue, CellId, Coverage, DependencySet, EntryFamilies, FamilyMember,
    Init, LocalState, OWN_SLOT, OrderKey, Origin, PublicationCarriers, Replacement, RootSet,
    SmallSet, StoreReceipt, TypedStore, TypedWrite, ValueAlt, ValueAlternatives, ValueKind,
    WrapperAge, WrapperId, remap_invoke_capture, remap_value_alternatives,
};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{ControlFlow, RangeInclusive};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in super::super) struct AllocId(u32);

impl AllocId {
    pub(in super::super) const MIN: Self = Self(0);
    pub(in super::super) const MAX: Self = Self(u32::MAX);

    /// Marker held by a (holder, field) group that is known to hold nothing.
    const EMPTY: AllocId = AllocId(u32::MAX);
}

/// One step from a holder to a value it stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in super::super) enum FieldKey {
    Field(u32),
    /// Ordinal for a reachable concrete type without a caller-local HIR layout.
    UnmaterializedField(u32),
    Payload {
        variant: u32,
        field: u32,
    },
    /// All collection elements or all map values.
    Elem,
    MapKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum AllocBase {
    /// The allocation a binding currently owns or observes through its roots.
    Root(u32),
    /// A value built by one construction expression.
    Site(u32),
    /// A proven fresh allocation in a specific call result construction.
    Call(OrderKey, bool, u32),
    /// Any unproven allocation of the given semantic type returned by one call.
    OpaqueCall(OrderKey, bool, u32, u32),
    /// A correlated value occurrence. It is not an allocation in the reachability graph.
    CallView(OrderKey, bool, u32),
    /// A binding's previous allocation, renamed when the binding was rebound at an order key.
    Old(u32, OrderKey),
    /// A binding's allocation after the binding left lexical scope.
    Departed(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AllocKey {
    Base(AllocBase),
    Child(AllocId, FieldKey),
    /// A node of one copy evaluation. Only acyclic sites retain source identities.
    Copied(AllocId, AllocId),
    CopiedOccurrence(AllocId, AllocId),
    CopyPair(AllocId, AllocId, AllocId),
}

#[derive(Debug, Clone)]
struct AllocInfo {
    key: AllocKey,
    base: AllocBase,
    /// Whether this id names at most one allocation at a time, so a write to one of its fields
    /// may replace that field's contents.
    singular: bool,
}

/// Per-function interner shared by every state of one analysis.
#[cfg_attr(test, derive(Clone))]
#[derive(Debug, Default)]
pub(in super::super) struct AllocTable {
    infos: Vec<AllocInfo>,
    index: FxHashMap<AllocKey, AllocId>,
    cyclic_blocks: Option<RecurringBlocks>,
}

#[derive(Debug)]
#[cfg_attr(test, derive(Clone))]
enum RecurringBlocks {
    Local(FxHashSet<BlockId>),
    Shared(Rc<FxHashSet<BlockId>>),
}

impl AllocTable {
    fn intern(&mut self, key: AllocKey, base: AllocBase, singular: bool) -> AllocId {
        if let Some(id) = self.index.get(&key) {
            return *id;
        }
        let id = AllocId(self.infos.len() as u32);
        self.infos.push(AllocInfo {
            key,
            base,
            singular,
        });
        self.index.insert(key, id);
        id
    }

    fn base_id(&mut self, base: AllocBase, singular: bool) -> AllocId {
        self.intern(AllocKey::Base(base), base, singular)
    }

    fn root(&mut self, local: usize) -> AllocId {
        self.base_id(AllocBase::Root(local as u32), true)
    }

    /// The lazy child of `parent` under `key`.
    fn child(&mut self, parent: AllocId, key: FieldKey) -> AllocId {
        let info = &self.infos[parent.0 as usize];
        let base = info.base;
        // Every element of one collection shares a key, so an element id is a summary.
        let singular = info.singular && !matches!(key, FieldKey::Elem | FieldKey::MapKey);
        self.intern(AllocKey::Child(parent, key), base, singular)
    }

    fn block_may_repeat(
        &mut self,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> bool {
        match self
            .cyclic_blocks
            .get_or_insert_with(|| RecurringBlocks::Local(cyclic_blocks(successors)))
        {
            RecurringBlocks::Local(blocks) => blocks.contains(&block),
            RecurringBlocks::Shared(blocks) => blocks.contains(&block),
        }
    }

    fn copied_node(&mut self, graph: AllocId, source: AllocId, singular: bool) -> AllocId {
        self.intern(AllocKey::Copied(graph, source), self.base(graph), singular)
    }

    fn parent(&self, id: AllocId) -> Option<(AllocId, FieldKey)> {
        match self.infos[id.0 as usize].key {
            AllocKey::Child(parent, key) => Some((parent, key)),
            AllocKey::Base(_)
            | AllocKey::Copied(..)
            | AllocKey::CopiedOccurrence(..)
            | AllocKey::CopyPair(..) => None,
        }
    }

    fn base(&self, id: AllocId) -> AllocBase {
        self.infos[id.0 as usize].base
    }

    fn singular(&self, id: AllocId) -> bool {
        self.infos[id.0 as usize].singular
    }

    /// `id` with its base replaced by `to`, keeping the field path.
    fn rebase(&mut self, id: AllocId, to: AllocBase, to_singular: bool) -> AllocId {
        if let Some((parent, key)) = self.parent(id) {
            let parent = self.rebase(parent, to, to_singular);
            self.child(parent, key)
        } else {
            self.base_id(to, to_singular)
        }
    }
}

/// One explicit holder edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct Edge {
    holder: AllocId,
    pub key: FieldKey,
    held: AllocId,
    /// Whether a mutation may pass through this edge. A value stored from an immutable binding
    /// keeps that binding's permission.
    writable: bool,
    /// Whether this group holds this allocation at most once.
    pub single: bool,
}

impl Edge {
    fn sort_key(&self) -> (AllocId, FieldKey, AllocId) {
        (self.holder, self.key, self.held)
    }

    pub(in super::super) fn value_alternative(&self) -> ValueAlt {
        ValueAlt::plain(stored_value_kind(self.held, self.writable))
    }
}

fn stored_value_kind(id: AllocId, writable: bool) -> ValueKind {
    if id == AllocId::EMPTY {
        ValueKind::Absent
    } else {
        ValueKind::Allocation(StoredValue { id, writable })
    }
}

fn holder_group_range(holder: AllocId) -> RangeInclusive<(AllocId, FieldKey)> {
    (holder, FieldKey::Field(0))..=(holder, FieldKey::MapKey)
}

fn paired_allocations(
    pairs: &BTreeSet<(AllocId, AllocId)>,
    first: AllocId,
) -> impl Iterator<Item = AllocId> + '_ {
    pairs
        .range((first, AllocId::MIN)..=(first, AllocId::MAX))
        .map(|&(_, second)| second)
}

fn remove_hash_bucket_member<K: Eq + std::hash::Hash, V: Ord>(
    buckets: &mut FxHashMap<K, BTreeSet<V>>,
    key: K,
    member: &V,
) {
    if let std::collections::hash_map::Entry::Occupied(mut bucket) = buckets.entry(key) {
        bucket.get_mut().remove(member);
        if bucket.get().is_empty() {
            bucket.remove();
        }
    }
}

fn index_base_rows<T: Copy + Ord>(
    table: &AllocTable,
    index: &mut BTreeSet<(AllocBase, T)>,
    owner: T,
    ids: impl IntoIterator<Item = AllocId>,
    insert: bool,
) {
    for id in ids.into_iter().filter(|&id| id != AllocId::EMPTY) {
        let row = (table.base(id), owner);
        if insert {
            index.insert(row);
        } else {
            index.remove(&row);
        }
    }
}

fn value_allocations(value: &ValueAlt) -> impl Iterator<Item = AllocId> + '_ {
    let stored = match value.kind {
        ValueKind::Allocation(stored) => Some(stored.id),
        _ => None,
    };
    stored.into_iter().chain(
        value
            .deps
            .iter()
            .flat_map(|dep| dep.entries.as_slice().iter().copied()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Observation {
    local: usize,
    values: Vec<StoredValue>,
    views: Vec<AllocId>,
    alternatives: ValueAlternatives,
    readonly: bool,
}

/// Projected holders can overlap across bases; a different selector never overlaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObservationRoute {
    Incomplete,
    Slot(WrapperId),
    Selector(FieldKey),
}

fn observation_routes(address: &AddressSet) -> BTreeSet<ObservationRoute> {
    if address.coverage != Coverage::Complete {
        return [ObservationRoute::Incomplete].into_iter().collect();
    }
    address
        .cells()
        .iter()
        .map(|cell| match *cell {
            CellId::Slot(slot) => ObservationRoute::Slot(slot),
            CellId::Projected(_, key) => ObservationRoute::Selector(key),
        })
        .collect()
}

struct ResolvedCells {
    addresses: AddressSet,
    projected_origins: SmallSet<Origin>,
    readonly_source: Option<usize>,
    cyclic: bool,
    prior: bool,
}

struct ResolutionFrame<'a> {
    wrapper: WrapperId,
    alternatives: std::slice::Iter<'a, BindingAlt>,
    result: ResolvedCells,
    aliases_only: bool,
}

impl ResolvedCells {
    fn include(&mut self, suffix: &Self) {
        if suffix.addresses.coverage == Coverage::Partial {
            self.addresses.coverage = Coverage::Partial;
        }
        self.addresses.all_candidates_known &= suffix.addresses.all_candidates_known;
        self.addresses
            .routes
            .extend_from_slice(&suffix.addresses.routes);
        self.addresses.routes.sort_unstable();
        self.addresses.routes.dedup();
        for &cell in suffix.addresses.cells() {
            self.addresses.cells.insert(cell);
        }
        for &origin in suffix.projected_origins.as_slice() {
            self.projected_origins.insert(origin);
        }
        self.readonly_source = self.readonly_source.or(suffix.readonly_source);
        self.cyclic |= suffix.cyclic;
        self.prior |= suffix.prior;
    }
}

/// Explicit rows own storage facts. The indices contain only direct links.
#[derive(Debug, Default)]
struct EdgeRows {
    groups: BTreeMap<(AllocId, FieldKey), Vec<Edge>>,
    by_base: FxHashMap<AllocBase, BTreeSet<(AllocId, FieldKey)>>,
    children: BTreeSet<(AllocId, AllocId)>,
    incoming: BTreeSet<(AllocId, AllocId, FieldKey)>,
}

impl Clone for EdgeRows {
    fn clone(&self) -> Self {
        add_frontend_counter(
            FrontendCounter::BorrowReachIndexMaintenanceRows,
            self.incidence_row_count(),
        );
        Self {
            groups: self.groups.clone(),
            by_base: self.by_base.clone(),
            children: self.children.clone(),
            incoming: self.incoming.clone(),
        }
    }
}

impl EdgeRows {
    fn iter(&self) -> impl Iterator<Item = &Edge> {
        self.groups.values().flatten()
    }

    fn incidence_row_count(&self) -> usize {
        self.children.len()
            + self.incoming.len()
            + self.by_base.values().map(BTreeSet::len).sum::<usize>()
    }

    #[cfg(feature = "benchmark_counters")]
    fn stored_row_count(&self) -> usize {
        self.groups.values().map(Vec::len).sum::<usize>() + self.incidence_row_count()
    }

    fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    fn holder_groups(&self, id: AllocId) -> impl Iterator<Item = &[Edge]> {
        self.groups
            .range(holder_group_range(id))
            .map(|(_, rows)| rows.as_slice())
    }

    fn for_holder(&self, id: AllocId) -> impl Iterator<Item = &Edge> {
        self.holder_groups(id).flatten()
    }

    fn group(&self, holder: AllocId, key: FieldKey) -> &[Edge] {
        self.groups.get(&(holder, key)).map_or(&[], Vec::as_slice)
    }

    fn has_outgoing(&self, holder: AllocId) -> bool {
        self.holder_groups(holder).next().is_some() || self.children_of(holder).next().is_some()
    }

    fn children_of(&self, parent: AllocId) -> impl Iterator<Item = AllocId> + '_ {
        paired_allocations(&self.children, parent)
    }

    fn incoming_holders(&self, held: AllocId) -> impl Iterator<Item = AllocId> + '_ {
        self.incoming
            .range(
                (held, AllocId::MIN, FieldKey::Field(0))..=(held, AllocId::MAX, FieldKey::MapKey),
            )
            .map(|&(_, holder, _)| holder)
    }

    fn index_edge(&mut self, table: &AllocTable, edge: Edge) {
        increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
        let group = (edge.holder, edge.key);
        let holder_base = table.base(edge.holder);
        self.by_base.entry(holder_base).or_default().insert(group);
        if edge.held != AllocId::EMPTY {
            self.incoming.insert((edge.held, edge.holder, edge.key));
            let base = table.base(edge.held);
            if base != holder_base {
                increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
                self.by_base.entry(base).or_default().insert(group);
            }
        }
        let mut current = edge.holder;
        while let Some((parent, _)) = table.parent(current) {
            increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
            // An existing link also proves that its whole prefix has already been indexed.
            if !self.children.insert((parent, current)) {
                break;
            }
            current = parent;
        }
    }

    fn prune_path(&mut self, table: &AllocTable, mut holder: AllocId) {
        while let Some((parent, _)) = table.parent(holder) {
            increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
            if self.has_outgoing(holder) {
                break;
            }
            self.children.remove(&(parent, holder));
            holder = parent;
        }
    }

    fn insert(&mut self, table: &AllocTable, edge: Edge, distinct_occurrence: bool) -> bool {
        let rows = self.groups.entry((edge.holder, edge.key)).or_default();
        match rows.binary_search_by_key(&edge.held, |row| {
            increment_frontend_counter(FrontendCounter::BorrowPublisherGroupRowsInspected);
            row.held
        }) {
            Ok(position) => {
                let existing = &mut rows[position];
                let before = *existing;
                existing.writable &= edge.writable;
                // Copied facts describe the same already-counted occurrence.
                existing.single &= edge.single && !distinct_occurrence;
                *existing != before
            }
            Err(position) => {
                let moved = if rows.len() == rows.capacity() {
                    rows.len()
                } else {
                    rows.len() - position
                };
                add_frontend_counter(FrontendCounter::BorrowPublisherGroupRowsMoved, moved + 1);
                rows.insert(position, edge);
                self.index_edge(table, edge);
                true
            }
        }
    }

    fn take_group(&mut self, table: &AllocTable, holder: AllocId, key: FieldKey) -> Vec<Edge> {
        let Some(rows) = self.groups.remove(&(holder, key)) else {
            return Vec::new();
        };
        let group = (holder, key);
        for edge in &rows {
            increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
            if edge.held != AllocId::EMPTY {
                self.incoming.remove(&(edge.held, holder, key));
                remove_hash_bucket_member(&mut self.by_base, table.base(edge.held), &group);
            }
        }
        remove_hash_bucket_member(&mut self.by_base, table.base(holder), &group);
        self.prune_path(table, holder);
        rows
    }

    fn remove_group(
        &mut self,
        table: &RefCell<AllocTable>,
        holder: AllocId,
        key: FieldKey,
        empty_only: bool,
    ) {
        if empty_only {
            let Some(rows) = self.groups.get_mut(&(holder, key)) else {
                return;
            };
            increment_frontend_counter(FrontendCounter::BorrowPublisherGroupRowsInspected);
            if !rows.last().is_some_and(|row| row.held == AllocId::EMPTY) {
                return;
            }
            if rows.len() > 1 {
                rows.pop();
                return;
            }
        }
        let rows = self.take_group(&table.borrow(), holder, key);
        add_frontend_counter(
            FrontendCounter::BorrowPublisherGroupRowsInspected,
            rows.len(),
        );
    }

    fn rebase(
        &mut self,
        table: &mut AllocTable,
        only: Option<AllocBase>,
        replacement: &impl Fn(AllocBase) -> Option<AllocBase>,
        rename: impl Fn(AllocId, &mut AllocTable) -> AllocId,
    ) {
        let mut affected = BTreeSet::new();
        if let Some(base) = only {
            increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
            if let Some(groups) = self.by_base.get(&base) {
                add_frontend_counter(
                    FrontendCounter::BorrowReachIndexMaintenanceRows,
                    groups.len(),
                );
                affected.extend(groups);
            }
        } else {
            for (&base, groups) in &self.by_base {
                increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
                if replacement(base).is_some() {
                    add_frontend_counter(
                        FrontendCounter::BorrowReachIndexMaintenanceRows,
                        groups.len(),
                    );
                    affected.extend(groups);
                }
            }
        }
        // Remove in the original coordinates before merging any colliding destinations.
        let mut moved = Vec::new();
        for (holder, key) in affected {
            moved.extend(self.take_group(table, holder, key));
        }
        for mut edge in moved {
            edge.holder = rename(edge.holder, table);
            edge.held = rename(edge.held, table);
            edge.single &= edge.held == AllocId::EMPTY || table.singular(edge.held);
            self.insert(table, edge, true);
        }
    }
}

#[derive(Debug, Default)]
struct SharingPairs {
    rows: Vec<(AllocId, AllocId, bool)>,
    positive_by_right: BTreeSet<(AllocId, AllocId)>,
}

impl Clone for SharingPairs {
    fn clone(&self) -> Self {
        add_frontend_counter(
            FrontendCounter::BorrowReachIndexMaintenanceRows,
            self.positive_by_right.len(),
        );
        Self {
            rows: self.rows.clone(),
            positive_by_right: self.positive_by_right.clone(),
        }
    }
}

impl PartialEq for SharingPairs {
    fn eq(&self, other: &Self) -> bool {
        self.rows == other.rows
    }
}

impl Eq for SharingPairs {}

impl SharingPairs {
    fn iter(&self) -> impl Iterator<Item = &(AllocId, AllocId, bool)> {
        self.rows.iter()
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[cfg(feature = "benchmark_counters")]
    fn stored_row_count(&self) -> usize {
        self.rows.len() + self.positive_by_right.len()
    }

    fn search(&self, pair: (AllocId, AllocId)) -> Result<usize, usize> {
        self.rows.binary_search_by_key(&pair, |&(a, b, _)| (a, b))
    }

    fn get(&self, left: AllocId, right: AllocId) -> Option<bool> {
        self.search(ordered_pair(left, right))
            .ok()
            .map(|index| self.rows[index].2)
    }

    fn nontrivial_positive(left: AllocId, right: AllocId, positive: bool) -> bool {
        positive && left != right
    }

    fn union(&mut self, left: AllocId, right: AllocId, may_share: bool) {
        let (left, right) = ordered_pair(left, right);
        match self.search((left, right)) {
            Ok(index) => self.rows[index].2 |= may_share,
            Err(index) => self.rows.insert(index, (left, right, may_share)),
        }
        if Self::nontrivial_positive(left, right, may_share) {
            increment_frontend_counter(FrontendCounter::BorrowReachIndexMaintenanceRows);
            self.positive_by_right.insert((right, left));
        }
    }

    fn remove(&mut self, left: AllocId, right: AllocId) -> bool {
        let index = self.search((left, right)).expect("indexed sharing pair");
        self.positive_by_right.remove(&(right, left));
        self.rows.remove(index).2
    }

    fn retain(&mut self, keep: impl FnMut(&(AllocId, AllocId, bool)) -> bool) {
        self.rows.retain(keep);
        add_frontend_counter(
            FrontendCounter::BorrowReachIndexMaintenanceRows,
            self.positive_by_right.len() + self.rows.len(),
        );
        self.positive_by_right.clear();
        self.positive_by_right
            .extend(self.rows.iter().filter_map(|&(left, right, positive)| {
                Self::nontrivial_positive(left, right, positive).then_some((right, left))
            }));
    }

    fn visit_incidence(
        &self,
        id: AllocId,
        mut each: impl FnMut((AllocId, AllocId)) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        increment_frontend_counter(FrontendCounter::BorrowOperandSharingRows);
        let start = self.rows.partition_point(|&(left, _, _)| {
            increment_frontend_counter(FrontendCounter::BorrowOperandSharingRows);
            left < id
        });
        for &(left, right, positive) in self.rows[start..]
            .iter()
            .take_while(|&&(left, _, _)| left == id)
        {
            increment_frontend_counter(FrontendCounter::BorrowOperandSharingRows);
            if Self::nontrivial_positive(left, right, positive) {
                each((left, right))?;
            }
        }
        for left in paired_allocations(&self.positive_by_right, id) {
            increment_frontend_counter(FrontendCounter::BorrowOperandSharingRows);
            each((left, id))?;
        }
        ControlFlow::Continue(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SharingRow {
    Member(AllocId, AllocId),
    Projection(AllocId, FieldKey, AllocId),
    Binding(WrapperId, AllocId),
    Pair(AllocId, AllocId),
}

impl SharingRow {
    fn map_ids(self, mut map: impl FnMut(AllocId) -> AllocId) -> Self {
        match self {
            Self::Member(a, b) => Self::Member(map(a), map(b)),
            Self::Projection(a, key, b) => Self::Projection(map(a), key, map(b)),
            Self::Binding(owner, id) => Self::Binding(owner, map(id)),
            Self::Pair(a, b) => {
                let (a, b) = ordered_pair(map(a), map(b));
                Self::Pair(a, b)
            }
        }
    }
}

/// A view names an occurrence, not a new allocation. Its members give ordinary provenance,
/// while a projection can name a new occurrence even when both paths have the same members.
#[derive(Debug, Clone, Default)]
struct SharingLayer {
    members: BTreeSet<(AllocId, AllocId)>,
    member_views: BTreeSet<(AllocId, AllocId)>,
    projections: BTreeMap<(AllocId, FieldKey), BTreeSet<AllocId>>,
    bindings: Vec<(WrapperId, AllocId)>,
    pairs: SharingPairs,
    by_base: BTreeMap<AllocBase, BTreeSet<SharingRow>>,
}

impl PartialEq for SharingLayer {
    fn eq(&self, other: &Self) -> bool {
        self.members == other.members
            && self.projections == other.projections
            && self.bindings == other.bindings
            && self.pairs == other.pairs
    }
}

impl Eq for SharingLayer {}

impl SharingLayer {
    fn index_row(
        by_base: &mut BTreeMap<AllocBase, BTreeSet<SharingRow>>,
        table: &AllocTable,
        row: SharingRow,
        insert: bool,
    ) {
        row.map_ids(|id| {
            if id != AllocId::EMPTY {
                let base = table.base(id);
                if insert {
                    by_base.entry(base).or_default().insert(row);
                } else if let Some(rows) = by_base.get_mut(&base) {
                    rows.remove(&row);
                    if rows.is_empty() {
                        by_base.remove(&base);
                    }
                }
            }
            id
        });
    }

    fn edit_row(&mut self, table: &AllocTable, row: SharingRow, share: bool, insert: bool) -> bool {
        Self::index_row(&mut self.by_base, table, row, insert);
        match (row, insert) {
            (SharingRow::Member(view, id), true) => self.insert_member(view, id),
            (SharingRow::Member(view, id), false) => {
                self.members.remove(&(view, id));
                self.member_views.remove(&(id, view));
            }
            (SharingRow::Projection(holder, key, held), true) => {
                return self
                    .projections
                    .entry((holder, key))
                    .or_default()
                    .insert(held);
            }
            (SharingRow::Projection(holder, key, held), false) => {
                let group = self
                    .projections
                    .get_mut(&(holder, key))
                    .expect("indexed projection");
                group.remove(&held);
                if group.is_empty() {
                    self.projections.remove(&(holder, key));
                }
            }
            (SharingRow::Binding(owner, view), true) => {
                insert_fact(&mut self.bindings, (owner, view))
            }
            (SharingRow::Binding(owner, view), false) => {
                let index = self
                    .bindings
                    .binary_search(&(owner, view))
                    .expect("indexed view binding");
                self.bindings.remove(index);
            }
            (SharingRow::Pair(a, b), true) => self.pairs.union(a, b, share),
            (SharingRow::Pair(a, b), false) => return self.pairs.remove(a, b),
        }
        false
    }

    fn clear_projection(&mut self, table: &AllocTable, view: AllocId, key: FieldKey) {
        if let Some(removed) = self.projections.remove(&(view, key)) {
            add_frontend_counter(
                FrontendCounter::BorrowPublisherGroupRowsInspected,
                removed.len(),
            );
            for held in removed {
                Self::index_row(
                    &mut self.by_base,
                    table,
                    SharingRow::Projection(view, key, held),
                    false,
                );
            }
        }
    }

    fn retain_binding_owners(&mut self, table: &AllocTable, keep: impl Fn(WrapperId) -> bool) {
        self.bindings.retain(|&(slot, view)| {
            let keep = keep(slot);
            if !keep {
                Self::index_row(
                    &mut self.by_base,
                    table,
                    SharingRow::Binding(slot, view),
                    false,
                );
            }
            keep
        });
    }

    fn retain_live_views(&mut self, table: &AllocTable, live: &FxHashSet<AllocId>) {
        self.members.retain(|&(view, _)| live.contains(&view));
        self.member_views.retain(|&(_, view)| live.contains(&view));
        self.projections
            .retain(|&(holder, _), _| live.contains(&holder));
        self.pairs
            .retain(|&(a, b, _)| live.contains(&a) && live.contains(&b));
        self.rebuild_bases(table);
    }

    fn rebuild_bases(&mut self, table: &AllocTable) {
        self.by_base = self.rebuild_incidence(table);
    }

    fn rebuild_incidence(&self, table: &AllocTable) -> BTreeMap<AllocBase, BTreeSet<SharingRow>> {
        let mut by_base = BTreeMap::new();
        let rows = self
            .members
            .iter()
            .map(|&(a, b)| SharingRow::Member(a, b))
            .chain(self.projections.iter().flat_map(|(&(a, key), children)| {
                children
                    .iter()
                    .map(move |&b| SharingRow::Projection(a, key, b))
            }))
            .chain(
                self.bindings
                    .iter()
                    .map(|&(slot, view)| SharingRow::Binding(slot, view)),
            )
            .chain(self.pairs.iter().map(|&(a, b, _)| SharingRow::Pair(a, b)));
        for row in rows {
            Self::index_row(&mut by_base, table, row, true);
        }
        by_base
    }

    fn rebase(
        &mut self,
        table: &mut AllocTable,
        only: Option<AllocBase>,
        affected: impl Fn(AllocBase) -> bool,
        rename: impl Fn(AllocId, &mut AllocTable) -> AllocId,
    ) {
        let rows: BTreeSet<_> = if let Some(base) = only {
            self.by_base
                .get(&base)
                .into_iter()
                .flatten()
                .copied()
                .collect()
        } else {
            self.by_base
                .iter()
                .filter(|(base, _)| affected(**base))
                .flat_map(|(_, rows)| rows.iter().copied())
                .collect()
        };
        let moved: Vec<_> = rows
            .into_iter()
            .map(|row| {
                increment_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows);
                let shares = self.edit_row(table, row, false, false);
                (row, shares)
            })
            .collect();
        for (row, shares) in moved {
            let row = row.map_ids(|id| rename(id, table));
            self.edit_row(table, row, shares, true);
        }
    }

    fn members(&self, view: AllocId) -> impl Iterator<Item = AllocId> + '_ {
        paired_allocations(&self.members, view)
    }

    fn insert_member(&mut self, view: AllocId, id: AllocId) {
        if self.members.insert((view, id)) {
            self.member_views.insert((id, view));
        }
    }

    fn views_for(&self, id: AllocId) -> impl Iterator<Item = AllocId> + '_ {
        paired_allocations(&self.member_views, id)
    }

    fn projections(&self, view: AllocId, key: FieldKey) -> impl Iterator<Item = AllocId> + '_ {
        self.projections
            .get(&(view, key))
            .into_iter()
            .flatten()
            .copied()
    }

    fn projected_by(&self, view: AllocId) -> impl Iterator<Item = (FieldKey, AllocId)> + '_ {
        self.projections
            .range(holder_group_range(view))
            .flat_map(|(&(_, key), children)| children.iter().map(move |&child| (key, child)))
    }

    fn bindings(&self, local: WrapperId) -> impl Iterator<Item = AllocId> + '_ {
        let start = self.bindings.partition_point(|(key, _)| *key < local);
        self.bindings[start..]
            .iter()
            .take_while(move |(key, _)| *key == local)
            .map(|(_, view)| *view)
    }

    fn clear_bindings(&mut self, table: &AllocTable, local: WrapperId) {
        let range = slot_group(&self.bindings, local, |&(owner, _)| owner);
        if !range.is_empty() {
            for &(_, view) in &self.bindings[range.clone()] {
                Self::index_row(
                    &mut self.by_base,
                    table,
                    SharingRow::Binding(local, view),
                    false,
                );
            }
            drop(self.bindings.drain(range));
        }
    }

    fn possible_pair(&self, left: AllocId, right: AllocId) -> Option<bool> {
        self.pairs.get(left, right)
    }

    fn is_empty(&self) -> bool {
        self.members.is_empty()
            && self.projections.is_empty()
            && self.bindings.is_empty()
            && self.pairs.is_empty()
    }
}

fn ordered_pair(left: AllocId, right: AllocId) -> (AllocId, AllocId) {
    (left.min(right), left.max(right))
}

fn insert_fact<T: Ord>(facts: &mut Vec<T>, fact: T) {
    if let Err(index) = facts.binary_search(&fact) {
        facts.insert(index, fact);
    }
}

fn slot_group<T>(
    rows: &[T],
    slot: WrapperId,
    owner: impl Fn(&T) -> WrapperId,
) -> std::ops::Range<usize> {
    let start = rows.partition_point(|row| owner(row) < slot);
    let count = rows[start..]
        .iter()
        .take_while(|row| owner(row) == slot)
        .count();
    start..start + count
}

type BoundRow = (WrapperId, AllocId, bool);

/// Explicit heap and slot contents, plus the sparse wrapper relation.
#[derive(Debug, Clone)]
pub(in super::super) struct AllocationLayer {
    table: Rc<RefCell<AllocTable>>,
    edges: EdgeRows,
    /// A missing current slot payload is described by its ordinary value roots.
    bound: BTreeMap<WrapperId, Vec<BoundRow>>,
    bound_by_base: BTreeSet<(AllocBase, WrapperId)>,
    /// Exceptional contents, by cell; dependency-free allocation edges stay compact.
    contents: BTreeMap<CellId, ValueAlternatives>,
    cells_by_base: FxHashMap<AllocBase, BTreeSet<CellId>>,
    wrappers: Vec<BindingRow>,
    wrapper_users: BTreeSet<(WrapperId, WrapperId)>,
    wrappers_by_base: BTreeSet<(AllocBase, WrapperId)>,
    slot_root_users: BTreeSet<(usize, usize)>,
    route_rows: usize,
    observations: BTreeMap<usize, Observation>,
    observations_by_base: BTreeSet<(AllocBase, usize)>,
    observation_routes: BTreeMap<usize, BTreeSet<ObservationRoute>>,
    observations_by_route: BTreeSet<(ObservationRoute, usize)>,
    /// (Result binding, former holder) pairs whose reachable graphs are disjoint on every path.
    detached: BTreeSet<(u32, AllocId)>,
    detached_by_base: BTreeSet<(AllocBase, u32, AllocId)>,
    sharing: SharingLayer,
}

impl PartialEq for AllocationLayer {
    fn eq(&self, other: &Self) -> bool {
        self.edges.groups == other.edges.groups
            && self.bound == other.bound
            && self.contents == other.contents
            && self.wrappers == other.wrappers
            && self.observations == other.observations
            && self.detached == other.detached
            && self.sharing == other.sharing
    }
}

impl Eq for AllocationLayer {}

impl AllocationLayer {
    pub(super) fn new() -> Self {
        Self {
            table: Rc::new(RefCell::new(AllocTable::default())),
            edges: EdgeRows::default(),
            bound: BTreeMap::new(),
            bound_by_base: BTreeSet::new(),
            contents: BTreeMap::new(),
            cells_by_base: FxHashMap::default(),
            wrappers: Vec::new(),
            wrapper_users: BTreeSet::new(),
            wrappers_by_base: BTreeSet::new(),
            slot_root_users: BTreeSet::new(),
            route_rows: 0,
            observations: BTreeMap::new(),
            observations_by_base: BTreeSet::new(),
            observation_routes: BTreeMap::new(),
            observations_by_route: BTreeSet::new(),
            detached: BTreeSet::new(),
            detached_by_base: BTreeSet::new(),
            sharing: SharingLayer::default(),
        }
    }

    fn is_empty(&self) -> bool {
        self.edges.is_empty()
            && self.bound.is_empty()
            && self.contents.is_empty()
            && self.wrappers.is_empty()
            && self.observations.is_empty()
            && self.detached.is_empty()
            && self.sharing.is_empty()
    }

    #[cfg(feature = "benchmark_counters")]
    pub(super) fn row_count(&self) -> usize {
        self.edges.stored_row_count()
            + self.bound.values().map(Vec::len).sum::<usize>()
            + self.bound_by_base.len()
            + self.contents.values().map(Vec::len).sum::<usize>()
            + self.wrappers.len()
            + self.wrapper_users.len()
            + self.wrappers_by_base.len()
            + self.slot_root_users.len()
            + self
                .cells_by_base
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
            + self.observations.len()
            + self.observations_by_base.len()
            + self.observations_by_route.len() * 2
            + self.detached.len()
            + self.detached_by_base.len()
            + self.sharing.members.len()
            + self.sharing.member_views.len()
            + self
                .sharing
                .projections
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
            + self.sharing.bindings.len()
            + self.sharing.pairs.stored_row_count()
            + self
                .sharing
                .by_base
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
    }

    fn edit_wrapper_incidence(
        users: &mut BTreeSet<(WrapperId, WrapperId)>,
        route_rows: &mut usize,
        row: &BindingRow,
        insert: bool,
    ) {
        if !row.routes.is_empty() {
            if insert {
                *route_rows += 1;
            } else {
                *route_rows -= 1;
            }
        }
        let mut record = |target: WrapperId| {
            increment_frontend_counter(FrontendCounter::BorrowRetirementWrapperRows);
            if insert {
                users.insert((target, row.wrapper));
            } else {
                users.remove(&(target, row.wrapper));
            }
        };
        for alt in row.alts.as_slice() {
            if let BindingTarget::Link(target) = alt.target {
                record(target);
            }
            if let Some(origin) = alt.origin {
                record(origin.wrapper);
                if let Some(owner) = origin.owner {
                    record(owner);
                }
            }
        }
    }

    fn wrapper_users(&self, target: WrapperId) -> impl Iterator<Item = WrapperId> + '_ {
        let min = WrapperId::current(0);
        let max = WrapperId::current(usize::MAX).retired(|_| true);
        increment_frontend_counter(FrontendCounter::BorrowRetirementWrapperRows);
        self.wrapper_users
            .range((target, min)..=(target, max))
            .map(|&(_, user)| {
                increment_frontend_counter(FrontendCounter::BorrowRetirementWrapperRows);
                user
            })
    }

    fn replace_wrappers(&mut self, rows: Vec<BindingRow>) {
        self.wrappers = rows;
        self.rebuild_wrapper_incidence();
    }

    fn retain_wrappers(&mut self, keep: impl Fn(WrapperId) -> bool) {
        let table = self.table.borrow();
        self.wrappers.retain(|row| {
            let keep = keep(row.wrapper);
            if !keep {
                Self::edit_wrapper_incidence(
                    &mut self.wrapper_users,
                    &mut self.route_rows,
                    row,
                    false,
                );
                Self::index_wrapper_slot_roots(&mut self.slot_root_users, row, false);
                Self::index_wrapper_allocations(&table, &mut self.wrappers_by_base, row, false);
            }
            keep
        });
    }

    fn rebuild_wrapper_incidence(&mut self) {
        self.wrapper_users.clear();
        self.slot_root_users.clear();
        self.wrappers_by_base.clear();
        let table = self.table.borrow();
        self.route_rows = 0;
        for row in &self.wrappers {
            Self::edit_wrapper_incidence(&mut self.wrapper_users, &mut self.route_rows, row, true);
            Self::index_wrapper_slot_roots(&mut self.slot_root_users, row, true);
            Self::index_wrapper_allocations(&table, &mut self.wrappers_by_base, row, true);
        }
    }

    fn index_wrapper_slot_roots(
        users: &mut BTreeSet<(usize, usize)>,
        row: &BindingRow,
        insert: bool,
    ) {
        if row.wrapper.age == WrapperAge::Current
            && row.has_slot()
            && let Some(roots) = &row.slot_roots
        {
            super::index_roots(users, row.wrapper.local, roots, insert);
        }
    }

    fn contents(&self, cell: CellId) -> &[ValueAlt] {
        increment_frontend_counter(FrontendCounter::BorrowRetirementCellRows);
        self.contents.get(&cell).map_or(&[], Vec::as_slice)
    }

    fn take_contents(&mut self, cell: CellId) -> Option<(CellId, ValueAlternatives)> {
        increment_frontend_counter(FrontendCounter::BorrowRetirementCellRows);
        let row = self.contents.remove_entry(&cell)?;
        Self::edit_cell_incidence(
            &self.table.borrow(),
            &mut self.cells_by_base,
            cell,
            &row.1,
            false,
        );
        Some(row)
    }

    fn edit_cell_incidence(
        table: &AllocTable,
        by_base: &mut FxHashMap<AllocBase, BTreeSet<CellId>>,
        cell: CellId,
        values: &[ValueAlt],
        insert: bool,
    ) {
        let mut record = |id| {
            if id == AllocId::EMPTY {
                return;
            }
            increment_frontend_counter(FrontendCounter::BorrowRetirementCellRows);
            let base = table.base(id);
            if insert {
                by_base.entry(base).or_default().insert(cell);
            } else {
                remove_hash_bucket_member(by_base, base, &cell);
            }
        };
        if let CellId::Projected(holder, _) = cell {
            record(holder);
        }
        values.iter().flat_map(value_allocations).for_each(record);
    }

    fn replace_contents(&mut self, contents: BTreeMap<CellId, ValueAlternatives>) {
        let table = self.table.borrow();
        self.cells_by_base.clear();
        for (&cell, values) in &contents {
            Self::edit_cell_incidence(&table, &mut self.cells_by_base, cell, values, true);
        }
        self.contents = contents;
    }

    fn retain_contents(&mut self, keep: impl Fn(CellId, &AllocTable) -> bool) {
        let table = self.table.borrow();
        self.contents.retain(|&cell, values| {
            let keeps = keep(cell, &table);
            if !keeps {
                Self::edit_cell_incidence(&table, &mut self.cells_by_base, cell, values, false);
            }
            keeps
        });
    }

    fn set_contents(&mut self, cell: CellId, mut alternatives: ValueAlternatives) {
        alternatives.sort_unstable();
        alternatives.dedup();
        self.take_contents(cell);
        if !alternatives.is_empty() {
            Self::edit_cell_incidence(
                &self.table.borrow(),
                &mut self.cells_by_base,
                cell,
                &alternatives,
                true,
            );
            self.contents.insert(cell, alternatives);
        }
    }

    fn project_views(
        &self,
        table: &mut AllocTable,
        ids: &[AllocId],
        views: &[AllocId],
        key: FieldKey,
    ) -> Vec<AllocId> {
        let roots = if views.is_empty() { ids } else { views };
        if !roots
            .iter()
            .any(|&root| self.sharing.projections(root, key).next().is_some())
        {
            return Vec::new();
        }
        let mut result = Vec::new();
        for &root in roots {
            let count = result.len();
            result.extend(self.sharing.projections(root, key));
            if count == result.len() {
                self.project_members(table, root, key, &mut result);
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }

    fn project_members(
        &self,
        table: &mut AllocTable,
        view: AllocId,
        key: FieldKey,
        ids: &mut Vec<AllocId>,
    ) {
        self.with_members(view, |holder| self.project_alloc(table, holder, key, ids));
    }

    fn project_alloc(
        &self,
        table: &mut AllocTable,
        holder: AllocId,
        key: FieldKey,
        ids: &mut Vec<AllocId>,
    ) {
        let group = self.edges.group(holder, key);
        if group.is_empty() {
            ids.push(table.child(holder, key));
        } else {
            ids.extend(
                group
                    .iter()
                    .filter_map(|edge| (edge.held != AllocId::EMPTY).then_some(edge.held)),
            );
        }
    }

    fn with_members(&self, view: AllocId, mut each: impl FnMut(AllocId)) {
        let mut members = self.sharing.members(view).peekable();
        if members.peek().is_none() {
            each(view);
        } else {
            members.for_each(each);
        }
    }

    fn views_may_share(&self, left: AllocId, right: AllocId) -> bool {
        if left == right {
            return true;
        }
        if let Some(share) = self.sharing.possible_pair(left, right) {
            return share;
        }
        let mut overlap = false;
        self.with_members(left, |a| {
            self.with_members(right, |b| overlap |= a == b);
        });
        overlap
    }

    fn bound_of(&self, local: WrapperId) -> &[(WrapperId, AllocId, bool)] {
        self.bound
            .get(&local)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn index_bound(
        table: &AllocTable,
        index: &mut BTreeSet<(AllocBase, WrapperId)>,
        rows: &[BoundRow],
        insert: bool,
    ) {
        for &(owner, id, _) in rows {
            index_base_rows(table, index, owner, [id], insert);
        }
    }

    fn set_bound(&mut self, owner: WrapperId, rows: Vec<BoundRow>) {
        let old = self.bound.remove(&owner);
        if old.is_none() && rows.is_empty() {
            return;
        }
        let table = self.table.borrow();
        if let Some(old) = old {
            Self::index_bound(&table, &mut self.bound_by_base, &old, false);
        }
        if !rows.is_empty() {
            Self::index_bound(&table, &mut self.bound_by_base, &rows, true);
            self.bound.insert(owner, rows);
        }
    }

    fn replace_bound(&mut self, mut rows: BTreeMap<WrapperId, Vec<BoundRow>>) -> bool {
        for group in rows.values_mut() {
            normalize_bindings(group);
        }
        if self.bound == rows {
            return false;
        }
        let table = self.table.borrow();
        self.bound_by_base.clear();
        for group in rows.values() {
            Self::index_bound(&table, &mut self.bound_by_base, group, true);
        }
        self.bound = rows;
        true
    }

    fn retain_bound(&mut self, keep: impl Fn(WrapperId) -> bool) {
        let table = self.table.borrow();
        self.bound.retain(|&slot, rows| {
            let keep = keep(slot);
            if !keep {
                Self::index_bound(&table, &mut self.bound_by_base, rows, false);
            }
            keep
        });
    }

    fn extend_bound(&mut self, owner: WrapperId, rows: impl IntoIterator<Item = BoundRow>) {
        let group = self.bound.entry(owner).or_default();
        let start = group.len();
        group.extend(rows);
        Self::index_bound(
            &self.table.borrow(),
            &mut self.bound_by_base,
            &group[start..],
            true,
        );
        normalize_bindings(group);
        if group.is_empty() {
            self.bound.remove(&owner);
        }
    }

    fn index_wrapper_allocations(
        table: &AllocTable,
        index: &mut BTreeSet<(AllocBase, WrapperId)>,
        row: &BindingRow,
        insert: bool,
    ) {
        let targets = row
            .alts
            .as_slice()
            .iter()
            .filter_map(|alt| match alt.target {
                BindingTarget::Projected(holder, _) => Some(holder),
                _ => None,
            });
        let routes = row.routes.iter().flat_map(|(cell, deps)| {
            let target = match cell {
                CellId::Projected(holder, _) => Some(*holder),
                _ => None,
            };
            target.into_iter().chain(
                deps.iter()
                    .flat_map(|dep| dep.entries.as_slice().iter().copied()),
            )
        });
        index_base_rows(table, index, row.wrapper, targets.chain(routes), insert);
    }

    fn observation(&self, local: usize) -> Option<&Observation> {
        self.observations.get(&local)
    }

    fn index_observation(
        table: &AllocTable,
        by_base: &mut BTreeSet<(AllocBase, usize)>,
        entry: &Observation,
        insert: bool,
    ) {
        let ids = entry
            .values
            .iter()
            .map(|value| value.id)
            .chain(entry.views.iter().copied())
            .chain(entry.alternatives.iter().flat_map(value_allocations));
        index_base_rows(table, by_base, entry.local, ids, insert);
    }

    fn set_observation(&mut self, local: usize, entry: Option<Observation>) {
        let old = self.observations.remove(&local);
        if old.is_none() && entry.is_none() {
            return;
        }
        self.remove_observation_routes(local);
        let table = self.table.borrow();
        if let Some(old) = old {
            Self::index_observation(&table, &mut self.observations_by_base, &old, false);
        }
        if let Some(entry) = entry {
            Self::index_observation(&table, &mut self.observations_by_base, &entry, true);
            self.observations.insert(local, entry);
        }
    }

    fn remove_observation_routes(&mut self, local: usize) {
        if let Some(old) = self.observation_routes.remove(&local) {
            for route in old {
                self.observations_by_route.remove(&(route, local));
            }
        }
    }

    fn set_observation_routes(&mut self, local: usize, routes: BTreeSet<ObservationRoute>) {
        self.remove_observation_routes(local);
        self.observations_by_route
            .extend(routes.iter().map(|&route| (route, local)));
        self.observation_routes.insert(local, routes);
    }

    fn clear_observations(&mut self) {
        self.observations.clear();
        self.observations_by_base.clear();
        self.observation_routes.clear();
        self.observations_by_route.clear();
    }

    fn retain_observations(&mut self, keep: impl Fn(&Observation) -> bool) {
        let removed: Vec<_> = self
            .observations
            .values()
            .filter(|entry| !keep(entry))
            .map(|entry| entry.local)
            .collect();
        for local in removed {
            self.set_observation(local, None);
        }
    }

    fn join_observations(
        &mut self,
        other: &Self,
        mut merge: impl FnMut(&mut Observation, &Observation) -> bool,
    ) -> bool {
        let table = self.table.borrow();
        let mut removed = Vec::new();
        let mut changed = false;
        let by_base = &mut self.observations_by_base;
        self.observations.retain(|&local, row| {
            let Some(incoming) = other.observation(local) else {
                Self::index_observation(&table, by_base, row, false);
                removed.push(local);
                return false;
            };
            if row != incoming {
                Self::index_observation(&table, by_base, row, false);
                changed |= merge(row, incoming);
                Self::index_observation(&table, by_base, row, true);
            }
            true
        });
        drop(table);
        changed |= !removed.is_empty();
        for local in removed {
            self.remove_observation_routes(local);
        }
        changed
    }

    fn projected_holders(&self) -> impl Iterator<Item = AllocId> + '_ {
        self.wrappers
            .iter()
            .flat_map(|row| row.alts.as_slice())
            .filter_map(|alt| match alt.target {
                BindingTarget::Projected(holder, _) => Some(holder),
                _ => None,
            })
    }

    fn add_edge(&mut self, edge: Edge) {
        self.insert_edge(edge, true);
    }

    fn insert_edge(&mut self, edge: Edge, distinct_occurrence: bool) {
        self.edges
            .insert(&self.table.borrow(), edge, distinct_occurrence);
    }

    /// Whether `descendant` is `ancestor` or a lazy descendant still held through every step.
    fn covers(&self, table: &AllocTable, ancestor: AllocId, descendant: AllocId) -> bool {
        self.covers_through(table, ancestor, descendant, false)
    }

    fn covers_through(
        &self,
        table: &AllocTable,
        ancestor: AllocId,
        descendant: AllocId,
        writable_only: bool,
    ) -> bool {
        let mut current = descendant;
        loop {
            if current == ancestor {
                return true;
            }
            let Some(parent) = self.covered_parent(table, current, writable_only, false) else {
                return false;
            };
            current = parent;
        }
    }

    fn covered_parent(
        &self,
        table: &AllocTable,
        id: AllocId,
        writable_only: bool,
        count_incidence: bool,
    ) -> Option<AllocId> {
        if count_incidence {
            increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
        }
        let (parent, key) = table.parent(id)?;
        let group = self.edges.group(parent, key);
        (group.is_empty()
            || group
                .binary_search_by_key(&id, |edge| {
                    if count_incidence {
                        increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
                    }
                    edge.held
                })
                .is_ok_and(|index| !writable_only || group[index].writable))
        .then_some(parent)
    }

    /// Name a source node reached by a lazy path from the nodes already in this copy.
    fn copied_path(
        &self,
        table: &mut AllocTable,
        copies: &mut FxHashMap<AllocId, AllocId>,
        site: AllocId,
        id: AllocId,
    ) -> Option<AllocId> {
        if let Some(&copy) = copies.get(&id) {
            return Some(copy);
        }
        let (parent, key) = table.parent(id)?;
        if !self.edges.group(parent, key).is_empty() {
            return None;
        }
        let copy_parent = self.copied_path(table, copies, site, parent)?;
        let copy = if table.singular(site) {
            table.copied_node(site, id, table.singular(id))
        } else {
            table.child(copy_parent, key)
        };
        copies.insert(id, copy);
        Some(copy)
    }

    fn copy_edges(
        &self,
        table: &mut AllocTable,
        sources: impl IntoIterator<Item = (AllocId, AllocId)>,
        site: AllocId,
        copied: &mut Vec<Edge>,
    ) {
        let singular = table.singular(site);
        let mut copies: FxHashMap<_, _> = sources.into_iter().collect();
        let mut pending: Vec<_> = self.edges.iter().collect();
        loop {
            let before = pending.len();
            pending.retain(|edge| {
                let Some(holder) = self.copied_path(table, &mut copies, site, edge.holder) else {
                    return true;
                };
                let held = if edge.held == AllocId::EMPTY {
                    AllocId::EMPTY
                } else if let Some(id) = self.copied_path(table, &mut copies, site, edge.held) {
                    id
                } else {
                    let id = if singular {
                        table.copied_node(site, edge.held, table.singular(edge.held))
                    } else {
                        table.child(holder, edge.key)
                    };
                    copies.insert(edge.held, id);
                    id
                };
                copied.push(Edge {
                    holder,
                    key: edge.key,
                    held,
                    writable: true,
                    single: edge.single && table.singular(holder),
                });
                false
            });
            if pending.len() == before {
                break;
            }
        }

        // A stored alias can discover a node before its lazy parent is reachable. Retain
        // that lazy route as well, including intermediate nodes with no group of their own.
        let mut parents: Vec<_> = copies
            .keys()
            .filter_map(|&id| table.parent(id))
            .filter_map(|(parent, key)| self.edges.group(parent, key).is_empty().then_some(parent))
            .collect();
        parents.sort_unstable();
        parents.dedup();
        for parent in parents {
            self.copied_path(table, &mut copies, site, parent);
        }
        for (&source, &held) in &copies {
            if let Some((parent, key)) = table.parent(source)
                && let Some(&holder) = copies.get(&parent)
                && self.edges.group(parent, key).is_empty()
            {
                copied.push(Edge {
                    holder,
                    key,
                    held,
                    writable: true,
                    single: false,
                });
            }
        }

        if singular {
            return;
        }

        // A repeated site and collapsed element paths name multiple graphs. Once a
        // group is explicit, retain the implicit child for alternatives that did not
        // record it. This applies to nested holders too, including wholly lazy arms.
        let base = table.base(site);
        let prior_instance = self.edges.iter().any(|edge| {
            table.base(edge.holder) == base
                || (edge.held != AllocId::EMPTY && table.base(edge.held) == base)
        }) || self
            .bound
            .values()
            .flatten()
            .any(|&(_, id, _)| id != AllocId::EMPTY && table.base(id) == base)
            || self
                .projected_holders()
                .any(|holder| table.base(holder) == base);
        let mut distinct = FxHashSet::default();
        if !prior_instance && copies.values().all(|&id| distinct.insert(id)) {
            return;
        }
        let mut groups: Vec<_> = copied.iter().map(|edge| (edge.holder, edge.key)).collect();
        groups.extend(self.edges.iter().filter_map(|edge| {
            (table.base(edge.holder) == base).then_some((edge.holder, edge.key))
        }));
        groups.sort_unstable();
        groups.dedup();
        for (holder, key) in groups {
            copied.push(Edge {
                holder,
                key,
                held: table.child(holder, key),
                writable: true,
                single: false,
            });
        }
    }

    /// Traverse occurrences rather than source IDs. A copied private path never acquires its
    /// neighbour's alternatives; a pair witness carries only its own common suffix route.
    fn copy_correlated_edges(
        &self,
        table: &mut AllocTable,
        site: AllocId,
        root: AllocId,
        view: AllocId,
        copied: &mut Vec<Edge>,
    ) {
        struct Route {
            copy: AllocId,
            ids: Vec<AllocId>,
            views: Vec<AllocId>,
            children: Vec<(FieldKey, Option<usize>)>,
        }
        let mut ids = Vec::new();
        self.with_members(view, |id| ids.push(id));
        let mut routes = vec![Route {
            copy: root,
            ids,
            views: vec![view],
            children: Vec::new(),
        }];
        let mut index = 0;
        while index < routes.len() {
            let route = &routes[index];
            let mut keys = Vec::new();
            // Include lazy ancestors of explicit holders and held allocations. A referenced
            // lazy child can share with another branch without ever having an explicit edge.
            for &source in &route.ids {
                for edge in self.edges.iter() {
                    for mut next in [edge.holder, edge.held] {
                        if next == AllocId::EMPTY || !self.covers(table, source, next) {
                            continue;
                        }
                        while let Some((parent, key)) = table.parent(next) {
                            if parent == source {
                                keys.push(key);
                                break;
                            }
                            next = parent;
                        }
                    }
                    if edge.holder == source {
                        keys.push(edge.key);
                    }
                }
            }
            for &view in &route.views {
                keys.extend(self.sharing.projected_by(view).map(|(key, _)| key));
            }
            keys.sort_unstable();
            keys.dedup();
            for key in keys {
                let route = &routes[index];
                let views = self.project_views(table, &route.ids, &route.views, key);
                let mut ids = Vec::new();
                if views.is_empty() {
                    for &source in &route.ids {
                        self.project_members(table, source, key, &mut ids);
                    }
                } else {
                    for &view in &views {
                        self.with_members(view, |id| ids.push(id));
                    }
                }
                ids.sort_unstable();
                ids.dedup();
                let child = if ids.is_empty() {
                    None
                } else {
                    let node = table.child(route.copy, key);
                    let offset = routes.len();
                    routes.push(Route {
                        copy: node,
                        ids,
                        views,
                        children: Vec::new(),
                    });
                    Some(offset)
                };
                routes[index].children.push((key, child));
            }
            index += 1;
        }

        let mut occurrences: FxHashMap<AllocId, Vec<usize>> = FxHashMap::default();
        let mut views: FxHashMap<AllocId, Vec<usize>> = FxHashMap::default();
        for (index, route) in routes.iter().enumerate() {
            for &id in &route.ids {
                occurrences.entry(id).or_default().push(index);
            }
            for &view in &route.views {
                views.entry(view).or_default().push(index);
            }
        }
        let mut candidates = FxHashSet::default();
        for nodes in occurrences.values() {
            for (index, &left) in nodes.iter().enumerate() {
                for &right in &nodes[index + 1..] {
                    candidates.insert((left, right));
                }
            }
        }
        for &(left, right, _) in self.sharing.pairs.iter().filter(|row| row.2) {
            if let (Some(left), Some(right)) = (views.get(&left), views.get(&right)) {
                for &left in left {
                    for &right in right {
                        if left != right {
                            candidates.insert((left.min(right), left.max(right)));
                        }
                    }
                }
            }
        }
        let mut candidates: Vec<_> = candidates.into_iter().collect();
        candidates.sort_unstable();
        let mut witnesses = FxHashMap::default();
        let mut at_route = vec![Vec::new(); routes.len()];
        for (left, right) in candidates {
            let (a, b) = (&routes[left], &routes[right]);
            let left_views = if a.views.is_empty() { &a.ids } else { &a.views };
            let right_views = if b.views.is_empty() { &b.ids } else { &b.views };
            if !left_views
                .iter()
                .any(|&a| right_views.iter().any(|&b| self.views_may_share(a, b)))
            {
                continue;
            }
            let (a, b) = ordered_pair(a.copy, b.copy);
            let witness = table.intern(AllocKey::CopyPair(site, a, b), table.base(site), false);
            witnesses.insert((left, right), witness);
            at_route[left].push(witness);
            at_route[right].push(witness);
        }
        let mut edge = |holder, key, held, single| {
            copied.push(Edge {
                holder,
                key,
                held,
                writable: true,
                single,
            });
        };
        for route in &routes {
            for &(key, child) in &route.children {
                if let Some(child) = child {
                    edge(route.copy, key, routes[child].copy, false);
                    for &witness in &at_route[child] {
                        edge(route.copy, key, witness, false);
                    }
                } else {
                    edge(route.copy, key, AllocId::EMPTY, true);
                }
            }
        }
        for ((left, right), witness) in witnesses.iter().map(|(&pair, &id)| (pair, id)) {
            for &(key, a) in &routes[left].children {
                let Ok(index) = routes[right]
                    .children
                    .binary_search_by_key(&key, |&(key, _)| key)
                else {
                    continue;
                };
                let b = routes[right].children[index].1;
                let matched = match (a, b) {
                    (Some(a), Some(b)) => witnesses.get(&(a.min(b), a.max(b))).copied(),
                    _ => None,
                };
                edge(witness, key, matched.unwrap_or(AllocId::EMPTY), false);
            }
        }
    }

    fn edit_detachment(
        facts: &mut BTreeSet<(u32, AllocId)>,
        by_base: &mut BTreeSet<(AllocBase, u32, AllocId)>,
        table: &AllocTable,
        key: (u32, AllocId),
        insert: bool,
    ) {
        let (owner, holder) = key;
        if insert {
            facts.insert(key);
        } else {
            facts.remove(&key);
        }
        if holder != AllocId::EMPTY {
            let incidence = (table.base(holder), owner, holder);
            if insert {
                by_base.insert(incidence);
            } else {
                by_base.remove(&incidence);
            }
        }
    }

    fn clear_detachments(&mut self, owner: usize) {
        let table = self.table.borrow();
        let owner = owner as u32;
        while let Some(&key) = self
            .detached
            .range((owner, AllocId::MIN)..=(owner, AllocId::MAX))
            .next()
        {
            Self::edit_detachment(
                &mut self.detached,
                &mut self.detached_by_base,
                &table,
                key,
                false,
            );
        }
    }

    fn retain_detachment_owners(&mut self, keep: impl Fn(u32) -> bool) {
        let removed: Vec<_> = self
            .detached
            .iter()
            .copied()
            .filter(|&(owner, _)| !keep(owner))
            .collect();
        let table = self.table.borrow();
        for key in removed {
            Self::edit_detachment(
                &mut self.detached,
                &mut self.detached_by_base,
                &table,
                key,
                false,
            );
        }
    }

    /// Rebase only allocations named by this layer, not every key in the shared interner.
    fn rename_bases(
        &mut self,
        only: Option<AllocBase>,
        replacement: impl Fn(AllocBase) -> Option<AllocBase>,
    ) {
        let mut table = self.table.borrow_mut();
        let detached = if let Some(base) = only {
            self.detached_by_base
                .range((base, 0, AllocId::MIN)..=(base, u32::MAX, AllocId::MAX))
                .map(|&(_, owner, holder)| (owner, holder))
                .collect::<Vec<_>>()
        } else {
            self.detached_by_base
                .iter()
                .filter(|&&(base, _, _)| replacement(base).is_some())
                .map(|&(_, owner, holder)| (owner, holder))
                .collect()
        };
        for key in detached {
            increment_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows);
            Self::edit_detachment(
                &mut self.detached,
                &mut self.detached_by_base,
                &table,
                key,
                false,
            );
        }
        let affected = |base| {
            increment_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows);
            replacement(base).is_some()
        };
        let has_edges = if let Some(base) = only {
            self.edges.by_base.contains_key(&base)
        } else {
            self.edges.by_base.keys().copied().any(affected)
        };
        let cells: BTreeSet<_> = if let Some(base) = only {
            self.cells_by_base
                .get(&base)
                .into_iter()
                .flatten()
                .copied()
                .collect()
        } else {
            self.cells_by_base
                .iter()
                .filter(|(base, _)| affected(**base))
                .flat_map(|(_, cells)| cells.iter().copied())
                .collect()
        };
        let observations: Vec<_> = if let Some(base) = only {
            self.observations_by_base
                .range((base, 0)..=(base, usize::MAX))
                .map(|&(_, local)| local)
                .collect()
        } else {
            self.observations_by_base
                .iter()
                .filter(|&&(base, _)| affected(base))
                .map(|&(_, local)| local)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        let sharing = only.map_or_else(
            || self.sharing.by_base.keys().copied().any(affected),
            |base| self.sharing.by_base.contains_key(&base),
        );
        let select = |index: &BTreeSet<(AllocBase, WrapperId)>| -> BTreeSet<_> {
            if let Some(base) = only {
                index
                    .range(
                        (base, WrapperId::current(0))
                            ..=(base, WrapperId::current(usize::MAX).retired(|_| true)),
                    )
                    .map(|&(_, owner)| owner)
                    .collect()
            } else {
                index
                    .iter()
                    .filter(|&&(base, _)| affected(base))
                    .map(|&(_, owner)| owner)
                    .collect()
            }
        };
        let bound = select(&self.bound_by_base);
        let wrappers = select(&self.wrappers_by_base);
        if !has_edges
            && cells.is_empty()
            && observations.is_empty()
            && !sharing
            && bound.is_empty()
            && wrappers.is_empty()
        {
            return;
        }
        let rename = |id: AllocId, table: &mut AllocTable| {
            if id != AllocId::EMPTY
                && let Some(to) = replacement(table.base(id))
            {
                return table.rebase(id, to, false);
            }
            id
        };
        if has_edges {
            self.edges.rebase(&mut table, only, &replacement, rename);
        }
        for owner in bound {
            let rows = self.bound.get_mut(&owner).expect("indexed slot payload");
            add_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows, rows.len());
            Self::index_bound(&table, &mut self.bound_by_base, rows, false);
            for (_, id, _) in rows.iter_mut() {
                *id = rename(*id, &mut table);
            }
            normalize_bindings(rows);
            Self::index_bound(&table, &mut self.bound_by_base, rows, true);
        }
        let mut pending = Vec::with_capacity(cells.len());
        for cell in cells {
            if let Some(values) = self.contents.remove(&cell) {
                Self::edit_cell_incidence(&table, &mut self.cells_by_base, cell, &values, false);
                pending.push((cell, values));
            }
        }
        for (cell, mut alternatives) in pending {
            let cell = cell.remap(|wrapper| wrapper, |id| rename(id, &mut table));
            remap_value_alternatives(&mut alternatives, |id| rename(id, &mut table));
            Self::edit_cell_incidence(&table, &mut self.cells_by_base, cell, &alternatives, true);
            match self.contents.entry(cell) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    merge_alternatives(entry.get_mut(), alternatives);
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(alternatives);
                }
            }
        }
        for owner in wrappers {
            increment_frontend_counter(FrontendCounter::BorrowRetirementWrapperRows);
            let index = self
                .wrappers
                .binary_search_by_key(&owner, |row| row.wrapper)
                .expect("indexed wrapper");
            let row = &mut self.wrappers[index];
            Self::index_wrapper_allocations(&table, &mut self.wrappers_by_base, row, false);
            row.alts.remap(|alt| match alt.target {
                BindingTarget::Projected(holder, key) => BindingAlt {
                    target: BindingTarget::Projected(rename(holder, &mut table), key),
                    ..alt
                },
                _ => alt,
            });
            // Both closures need the same mutable interner.
            for (cell, deps) in &mut row.routes {
                if let CellId::Projected(holder, _) = cell {
                    *holder = rename(*holder, &mut table);
                }
                super::remap_dependencies(deps, |id| rename(id, &mut table));
            }
            row.routes.sort_unstable();
            row.routes.dedup();
            Self::index_wrapper_allocations(&table, &mut self.wrappers_by_base, row, true);
        }
        for local in observations {
            increment_frontend_counter(FrontendCounter::BorrowRetirementCellRows);
            let entry = self
                .observations
                .get_mut(&local)
                .expect("indexed observation");
            Self::index_observation(&table, &mut self.observations_by_base, entry, false);
            for value in &mut entry.values {
                value.id = rename(value.id, &mut table);
            }
            for view in &mut entry.views {
                *view = rename(*view, &mut table);
            }
            remap_value_alternatives(&mut entry.alternatives, |id| rename(id, &mut table));
            normalize_observation(entry);
            Self::index_observation(&table, &mut self.observations_by_base, entry, true);
        }
        if sharing {
            self.sharing.rebase(&mut table, only, affected, rename);
        }
    }
}

fn normalize_bindings(bindings: &mut Vec<(WrapperId, AllocId, bool)>) {
    bindings.sort_unstable();
    bindings.dedup_by(|later, earlier| {
        if (later.0, later.1) != (earlier.0, earlier.1) {
            return false;
        }
        earlier.2 |= later.2;
        true
    });
}

fn normalize_observation(entry: &mut Observation) {
    entry.values.sort_unstable_by_key(|value| value.id);
    entry.values.dedup_by(|later, earlier| {
        if later.id != earlier.id {
            return false;
        }
        earlier.writable &= later.writable;
        true
    });
    entry.views.sort_unstable();
    entry.views.dedup();
}

fn inherit_dependencies(mut alt: ValueAlt, deps: &[super::ReceiverDependency]) -> ValueAlt {
    alt.deps.extend_from_slice(deps);
    alt.deps.sort_unstable();
    alt.deps.dedup();
    alt
}

fn append_projected_alternatives(
    alternatives: &mut ValueAlternatives,
    parents: &[ValueAlt],
    holder: AllocId,
    children: impl Iterator<Item = ValueAlt> + Clone,
) {
    if parents.is_empty() {
        alternatives.extend(children);
    } else {
        for parent in parents {
            if matches!(parent.kind, ValueKind::Allocation(value) if value.id == holder) {
                alternatives.extend(
                    children
                        .clone()
                        .map(|alt| inherit_dependencies(alt, &parent.deps)),
                );
            }
        }
    }
}

fn observed_alternatives(entry: &Observation, address: &AddressSet) -> ValueAlternatives {
    if address.retained_routes().is_empty() {
        return entry.alternatives.clone();
    }
    let implicit;
    let old = if entry.alternatives.is_empty() {
        implicit = compact_alternatives(entry.values.iter().copied());
        &implicit
    } else {
        &entry.alternatives
    };
    let mut result = Vec::new();
    for (_, deps) in address.retained_routes() {
        result.extend(
            old.iter()
                .cloned()
                .map(|alt| inherit_dependencies(alt, deps)),
        );
    }
    result.sort_unstable();
    result.dedup();
    result
}

fn merge_alternatives(into: &mut ValueAlternatives, incoming: impl IntoIterator<Item = ValueAlt>) {
    into.extend(incoming);
    into.sort_unstable();
    into.dedup();
}

fn compact_alternatives(values: impl IntoIterator<Item = StoredValue>) -> ValueAlternatives {
    let mut alternatives: Vec<_> = values
        .into_iter()
        .map(|value| ValueAlt::plain(ValueKind::Allocation(value)))
        .collect();
    if alternatives.is_empty() {
        alternatives.push(ValueAlt::plain(ValueKind::Scalar));
    }
    alternatives
}

/// Graph content only. Its alternatives say nothing about the cells that supplied them.
#[derive(Debug, Clone)]
pub(in super::super) struct ResolvedPlace {
    pub ids: Vec<AllocId>,
    /// Empty uses ordinary allocation identity. Otherwise these names retain path occurrence.
    pub views: Vec<AllocId>,
    /// Whether the ids depend on layer facts that root provenance does not carry: a binding
    /// override or an explicit holder edge.
    pub layer_only: bool,
    /// Whether the path passes through a value stored from an immutable binding.
    pub readonly: bool,
    /// The binding that stored value came from, when the layer still names it.
    pub readonly_source: Option<usize>,
    pub coverage: Coverage,
    /// A represented alternative with no allocation, not a missing graph fact.
    pub may_be_absent: bool,
    pub alternatives: ValueAlternatives,
}

impl ResolvedPlace {
    pub(in super::super) fn from_values(values: &[StoredValue], views: Vec<AllocId>) -> Self {
        let readonly = values.iter().any(|value| !value.writable);
        Self {
            ids: values.iter().map(|value| value.id).collect(),
            views,
            layer_only: false,
            readonly,
            readonly_source: None,
            coverage: Coverage::Complete,
            may_be_absent: false,
            alternatives: Vec::new(),
        }
    }

    pub(in super::super) fn from_capture(capture: &CapturedValue) -> Self {
        let mut resolved = Self::from_values(&capture.values, capture.views.clone());
        resolved.alternatives.clone_from(&capture.alternatives);
        resolved
    }

    /// A known absence is covered as content but cannot supply a holder.
    pub(in super::super) fn holder_coverage(&self) -> Coverage {
        if self.may_be_absent {
            Coverage::Partial
        } else {
            self.coverage
        }
    }
}

/// Permission to access the cell itself. Referent mutation also checks the loaded content.
#[derive(Debug, Clone, Copy)]
pub(in super::super) struct ResolvedPermission {
    pub wrapper_readonly_source: Option<usize>,
    pub holder_readonly: bool,
    pub holder_readonly_source: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(in super::super) struct AccessRoute {
    pub local: usize,
    pub projected: bool,
}

/// Resolve the storage first; only an observation needs to follow the final cell's payload.
#[derive(Debug, Clone)]
pub(in super::super) struct PlaceResolution {
    pub address: AddressSet,
    pub traversal: SmallSet<CellId>,
    pub traversal_coverage: Coverage,
    pub content: Option<ResolvedPlace>,
    pub permission: ResolvedPermission,
    pub route: AccessRoute,
    terminal_projection: Option<(ResolvedPlace, FieldKey)>,
    projected_origins: SmallSet<Origin>,
    base_owner: Option<WrapperId>,
}

impl PlaceResolution {
    pub(in super::super) fn final_projection(&self) -> Option<(&ResolvedPlace, FieldKey)> {
        self.terminal_projection
            .as_ref()
            .map(|(holders, key)| (holders, *key))
    }

    /// An occurrence participating as an actual write target is not an independent loan.
    pub(in super::super) fn names_terminal_slot(&self, slot: WrapperId) -> bool {
        !self.route.projected && self.address.cells().contains(&CellId::Slot(slot))
    }

    /// The definite slot through which the holder was reached, including through links.
    pub(in super::super) fn projection_owner(&self) -> Option<WrapperId> {
        if self.route.projected && self.base_owner.is_some() {
            return self.base_owner;
        }
        if self.address.coverage != Coverage::Complete
            || !self.route.projected
                && self
                    .address
                    .cells()
                    .iter()
                    .any(|cell| matches!(cell, CellId::Slot(_)))
        {
            return None;
        }
        let origins = self.projected_origins.as_slice();
        let owner = origins.first()?.owner?;
        origins
            .iter()
            .all(|origin| origin.owner == Some(owner))
            .then_some(owner)
    }

    pub(in super::super) fn captured_origin(&self) -> Option<Origin> {
        match self.projected_origins.as_slice() {
            [origin] if self.address.definite().is_some() => Some(*origin),
            _ => None,
        }
    }

    pub(in super::super) fn projected_holder_route(&self, actor: usize) -> Option<bool> {
        let mut matches = self
            .projected_origins
            .as_slice()
            .iter()
            .filter(|origin| origin.wrapper.local == actor);
        Some(matches.next()?.isolated && matches.all(|origin| origin.isolated))
    }
}

/// One allocation stored into a holder field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in super::super) struct StoredValue {
    pub id: AllocId,
    pub writable: bool,
}

/// Rebase captures of a previous binding, even when no sparse fact named it before the rebind.
#[derive(Clone, Copy)]
pub(in super::super) struct AllocationRetirement {
    local: usize,
    order: OrderKey,
    wrapper: bool,
}

impl AllocationRetirement {
    pub(in super::super) fn retires_local(self, local: usize) -> bool {
        self.local == local
    }

    pub(in super::super) fn remap_wrapper(self, wrapper: WrapperId) -> WrapperId {
        wrapper.retired(|local| self.wrapper && self.retires_local(local))
    }

    pub(in super::super) fn remap_id(self, state: &BorrowState, id: &mut AllocId) {
        *id = state.remap_retired_alloc_id(*id, self.order, |local| self.retires_local(local));
    }

    pub(in super::super) fn remap_value(self, state: &BorrowState, value: &mut StoredValue) {
        self.remap_id(state, &mut value.id);
    }
}

impl BorrowState {
    pub(super) fn binding_row(&self, wrapper: WrapperId) -> Option<&BindingRow> {
        self.layer
            .wrappers
            .binary_search_by_key(&wrapper, |row| row.wrapper)
            .ok()
            .map(|index| &self.layer.wrappers[index])
    }

    fn current_wrapper_init(&self, wrapper: WrapperId) -> Option<Init> {
        (wrapper.age == WrapperAge::Current).then(|| self.locals[wrapper.local].init)
    }

    fn wrapper_coverage(init: Option<Init>, explicit: Option<Coverage>) -> Coverage {
        match (init, explicit) {
            (Some(Init::Yes), None | Some(Coverage::Complete))
            | (None, Some(Coverage::Complete)) => Coverage::Complete,
            _ => Coverage::Partial,
        }
    }

    fn implicit_binding_alternatives(init: Option<Init>) -> &'static [BindingAlt] {
        if matches!(init, Some(Init::Yes | Init::Maybe)) {
            &[OWN_SLOT]
        } else {
            &[]
        }
    }

    #[allow(dead_code)] // Boundary export also visits wrappers no longer named by a local.
    pub(in super::super) fn boundary_binding(&self, wrapper: WrapperId) -> BoundaryBinding<'_> {
        let row = self.binding_row(wrapper);
        let init = self.current_wrapper_init(wrapper);
        BoundaryBinding {
            alternatives: row.map_or_else(
                || Self::implicit_binding_alternatives(init),
                |row| row.alts.as_slice(),
            ),
            coverage: Self::wrapper_coverage(init, row.map(|row| row.coverage)),
            routes: row.map_or(&[], |row| row.routes.as_slice()),
        }
    }

    pub(in super::super) fn has_wrapped_binding(&self, local: usize) -> bool {
        self.binding_row(WrapperId::current(local)).is_some()
    }

    pub(in super::super) fn wrapper_locals(&self) -> impl Iterator<Item = usize> + '_ {
        self.layer
            .wrappers
            .iter()
            .filter_map(|row| (row.wrapper.age == WrapperAge::Current).then_some(row.wrapper.local))
    }

    pub(in super::super) fn allocation_observer_locals(&self) -> impl Iterator<Item = usize> + '_ {
        self.layer
            .bound
            .keys()
            .filter_map(|slot| (slot.age == WrapperAge::Current).then_some(slot.local))
            .chain(self.layer.observations.keys().copied())
            // A lazy, unnamed owner can still reach an explicitly stored descendant.
            .chain(
                self.layer
                    .edges
                    .iter()
                    .filter_map(|edge| self.alloc_origin_local(edge.holder)),
            )
    }

    fn put_binding_row(&mut self, row: BindingRow) {
        let wrapper = row.wrapper;
        let layer = &mut self.layer;
        let table = layer.table.borrow();
        match layer
            .wrappers
            .binary_search_by_key(&row.wrapper, |row| row.wrapper)
        {
            Ok(index) => {
                let old = std::mem::replace(&mut layer.wrappers[index], row);
                AllocationLayer::index_wrapper_allocations(
                    &table,
                    &mut layer.wrappers_by_base,
                    &old,
                    false,
                );
                AllocationLayer::index_wrapper_allocations(
                    &table,
                    &mut layer.wrappers_by_base,
                    &layer.wrappers[index],
                    true,
                );
                AllocationLayer::index_wrapper_slot_roots(&mut layer.slot_root_users, &old, false);
                AllocationLayer::index_wrapper_slot_roots(
                    &mut layer.slot_root_users,
                    &layer.wrappers[index],
                    true,
                );
                AllocationLayer::edit_wrapper_incidence(
                    &mut layer.wrapper_users,
                    &mut layer.route_rows,
                    &old,
                    false,
                );
                AllocationLayer::edit_wrapper_incidence(
                    &mut layer.wrapper_users,
                    &mut layer.route_rows,
                    &layer.wrappers[index],
                    true,
                );
            }
            Err(index) => {
                AllocationLayer::index_wrapper_slot_roots(&mut layer.slot_root_users, &row, true);
                AllocationLayer::index_wrapper_allocations(
                    &table,
                    &mut layer.wrappers_by_base,
                    &row,
                    true,
                );
                AllocationLayer::edit_wrapper_incidence(
                    &mut layer.wrapper_users,
                    &mut layer.route_rows,
                    &row,
                    true,
                );
                layer.wrappers.insert(index, row);
            }
        }
        drop(table);
        self.refresh_linked_observations(wrapper);
    }

    pub(super) fn remove_current_row(&mut self, local: usize) {
        let wrapper = WrapperId::current(local);
        if let Ok(index) = self
            .layer
            .wrappers
            .binary_search_by_key(&wrapper, |row| row.wrapper)
        {
            let old = self.layer.wrappers.remove(index);
            AllocationLayer::index_wrapper_allocations(
                &self.layer.table.borrow(),
                &mut self.layer.wrappers_by_base,
                &old,
                false,
            );
            AllocationLayer::index_wrapper_slot_roots(&mut self.layer.slot_root_users, &old, false);
            AllocationLayer::edit_wrapper_incidence(
                &mut self.layer.wrapper_users,
                &mut self.layer.route_rows,
                &old,
                false,
            );
        }
        self.layer.set_observation(local, None);
        self.layer.take_contents(CellId::Slot(wrapper));
    }

    fn row_from_spec(&self, local: usize, spec: BindingSpec) -> Option<BindingRow> {
        let wrapper = WrapperId::current(local);
        let (alts, coverage, route_roots, routes) = match spec {
            BindingSpec::Slot => return None,
            BindingSpec::Link { source, writable } => (
                SmallSet::One(BindingAlt {
                    target: BindingTarget::Link(source),
                    writable,
                    origin: None,
                }),
                Coverage::Complete,
                RootSet::empty(self.locals.len()),
                Vec::new(),
            ),
            #[cfg(test)]
            BindingSpec::Alias {
                addresses,
                route_roots,
            } => (
                SmallSet::from_iter(addresses.cells().iter().map(|&cell| BindingAlt {
                    target: match cell {
                        CellId::Slot(source) => BindingTarget::Link(source),
                        CellId::Projected(holder, key) => BindingTarget::Projected(holder, key),
                    },
                    writable: true,
                    origin: None,
                })),
                addresses.coverage,
                route_roots,
                addresses.routes,
            ),
            BindingSpec::Projected {
                addresses,
                origin,
                owner,
                writable,
                isolated,
                route_roots,
            } => (
                SmallSet::from_iter(addresses.cells().iter().filter_map(|&cell| match cell {
                    CellId::Projected(holder, key) => Some(BindingAlt {
                        target: BindingTarget::Projected(holder, key),
                        writable,
                        origin: Some(Origin {
                            wrapper: origin,
                            owner,
                            isolated,
                        }),
                    }),
                    CellId::Slot(_) => None,
                })),
                addresses.coverage,
                route_roots,
                addresses.routes,
            ),
        };
        Some(BindingRow {
            wrapper,
            alts,
            coverage,
            route_roots,
            slot_roots: None,
            routes,
        })
    }

    pub(super) fn install_binding_spec(&mut self, local: usize, spec: BindingSpec) {
        if let Some(row) = self.row_from_spec(local, spec) {
            self.put_binding_row(row);
        }
    }

    #[allow(dead_code)]
    pub(in super::super) fn publish_captured_binding(
        &mut self,
        wrapper: WrapperId,
        alts: SmallSet<BindingAlt>,
        coverage: Coverage,
        routes: CapturedRoutes,
        replace: Replacement,
        route_roots: Option<RootSet>,
    ) -> PublicationCarriers {
        let mut receipt = PublicationCarriers::default();
        let cell = CellId::Slot(wrapper);
        let exact = replace == Replacement::Must && wrapper.age == WrapperAge::Current;
        let mut row = BindingRow {
            wrapper,
            alts,
            coverage,
            route_roots: route_roots.unwrap_or_else(|| RootSet::empty(self.locals.len())),
            slot_roots: None,
            routes,
        };
        if !exact {
            if let Some(old) = self.binding_row(wrapper) {
                row.union_row_alts(old);
                row.route_roots.union_with(&old.route_roots);
                row.slot_roots.clone_from(&old.slot_roots);
                if old.coverage == Coverage::Partial {
                    row.coverage = Coverage::Partial;
                }
            } else {
                row.union_alts(Self::implicit_binding_alternatives(
                    self.current_wrapper_init(wrapper),
                ));
            }
        } else if !row.has_slot() {
            receipt.record(cell, self.layer.contents(cell), cell, &[], true);
            self.clear_slot_payload(wrapper);
        }
        self.invalidate_observations(&[cell], coverage);
        if wrapper.age == WrapperAge::Current && wrapper.local < self.locals.len() {
            let mut roots = row.route_roots.clone();
            if !exact || row.has_slot() {
                roots.union_with(&self.locals[wrapper.local].value_roots);
            }
            self.set_local_value_roots(wrapper.local, roots);
            self.set_local_derivation(wrapper.local, RootSet::empty(self.locals.len()));
        }
        self.put_binding_row(row);
        receipt
    }

    pub(in super::super) fn transition_slot_to_alias(
        &mut self,
        local: usize,
        spec: BindingSpec,
        roots: RootSet,
        derivation: RootSet,
        preserve_slot: bool,
    ) {
        let Some(mut row) = self.row_from_spec(local, spec) else {
            return;
        };
        let wrapper = row.wrapper;
        self.invalidate_observations(&[CellId::Slot(wrapper)], Coverage::Complete);
        if let Some(old) = self.binding_row(wrapper) {
            row.union_row_alts(old);
            row.route_roots.union_with(&old.route_roots);
            if old.coverage == Coverage::Partial {
                row.coverage = Coverage::Partial;
            }
            let mut derivation = derivation;
            if self.binding(local).mode().contains(LocalMode::ALIAS) {
                derivation.intersect_with(&self.locals[local].derived_from_roots);
            }
            self.set_local_derivation(local, derivation);
        } else {
            self.set_local_derivation(local, derivation);
        }
        if preserve_slot {
            row.union_alts(&[OWN_SLOT]);
            row.slot_roots = Some(self.slot_roots(wrapper).clone());
            let mut merged = roots;
            merged.union_with(&self.locals[local].value_roots);
            self.set_local_value_roots(local, merged);
        } else {
            row.alts = SmallSet::from_iter(
                row.alts
                    .as_slice()
                    .iter()
                    .copied()
                    .filter(|alt| alt.target != BindingTarget::OwnSlot),
            );
            self.clear_slot_payload(wrapper);
            self.set_local_value_roots(local, roots);
        }
        self.put_binding_row(row);
    }

    pub(in super::super) fn mark_slot_derivation(&mut self, local: usize, mut roots: RootSet) {
        let mode = self.binding_mode(local);
        if mode.contains(LocalMode::SLOT) {
            if mode.contains(LocalMode::ALIAS) {
                roots.intersect_with(&self.locals[local].derived_from_roots);
            }
            self.set_local_derivation(local, roots);
        }
    }

    pub(in super::super) fn resolve_cells(&self, local: usize) -> AddressSet {
        self.resolve_wrapper(WrapperId::current(local), false, &[])
            .addresses
    }

    pub(in super::super) fn resolve_alias_cells(&self, local: usize) -> AddressSet {
        self.resolve_wrapper(WrapperId::current(local), true, &[])
            .addresses
    }

    fn resolve_wrapper(
        &self,
        wrapper: WrapperId,
        aliases_only: bool,
        local_mutable: &[bool],
    ) -> ResolvedCells {
        let readonly = |wrapper: WrapperId| {
            (local_mutable.get(wrapper.local) == Some(&false)).then_some(wrapper.local)
        };
        let Some(row) = self.binding_row(wrapper) else {
            return self.implicit_slot(wrapper, aliases_only, readonly(wrapper));
        };
        let mut active = FxHashSet::default();
        let mut completed: FxHashMap<_, ResolvedCells> = FxHashMap::default();
        let mut parents = Vec::new();
        let mut frame = self.resolution_frame(row, aliases_only, readonly(wrapper));
        loop {
            increment_frontend_counter(FrontendCounter::BorrowRetirementWrapperRows);
            if let Some(alt) = frame.alternatives.next() {
                let cell = match alt.target {
                    BindingTarget::OwnSlot if frame.aliases_only => continue,
                    BindingTarget::OwnSlot => CellId::Slot(frame.wrapper),
                    BindingTarget::Link(source) => {
                        if !alt.writable {
                            frame
                                .result
                                .readonly_source
                                .get_or_insert(frame.wrapper.local);
                        }
                        if let Some(suffix) = completed.get(&source) {
                            frame.result.include(suffix);
                        } else if let Some(row) = self.binding_row(source) {
                            if source != wrapper && active.insert(source) {
                                parents.push(frame);
                                frame = self.resolution_frame(row, false, readonly(source));
                            } else {
                                // Cutting a backedge must not suppress a different route to this row.
                                frame.result.addresses.coverage = Coverage::Partial;
                                frame.result.cyclic = true;
                            }
                        } else {
                            frame.result.include(&self.implicit_slot(
                                source,
                                false,
                                readonly(source),
                            ));
                        }
                        continue;
                    }
                    BindingTarget::Projected(holder, key) => {
                        if let Some(origin) = alt.origin {
                            frame.result.projected_origins.insert(origin);
                        }
                        if !alt.writable {
                            frame
                                .result
                                .readonly_source
                                .get_or_insert(frame.wrapper.local);
                        }
                        CellId::Projected(holder, key)
                    }
                };
                frame.result.addresses.cells.insert(cell);
                continue;
            }

            let suffix = frame.result;
            if let Some(parent) = parents.pop() {
                let suffix_wrapper = frame.wrapper;
                active.remove(&suffix_wrapper);
                frame = parent;
                frame.result.include(&suffix);
                if !suffix.cyclic {
                    // Completed suffixes exclude the restrictions of their incoming routes.
                    completed.insert(suffix_wrapper, suffix);
                }
            } else {
                let mut result = suffix;
                if !result.prior
                    && match result.addresses.cells() {
                        [CellId::Projected(holder, key)] => {
                            self.projected_cell_is_exact(*holder, *key)
                        }
                        _ => true,
                    }
                {
                    result.addresses = result.addresses.with_singleton_proof();
                }
                return result;
            }
        }
    }

    fn implicit_slot(
        &self,
        wrapper: WrapperId,
        aliases_only: bool,
        readonly_source: Option<usize>,
    ) -> ResolvedCells {
        let init = self.current_wrapper_init(wrapper);
        ResolvedCells {
            addresses: {
                let mut addresses = AddressSet::from_cells(
                    (!aliases_only && !Self::implicit_binding_alternatives(init).is_empty())
                        .then_some(CellId::Slot(wrapper)),
                    Self::wrapper_coverage(init, None),
                )
                .with_singleton_proof();
                // A missing Current binding is uninitialised, not an unmodelled address.
                addresses.all_candidates_known = wrapper.age == WrapperAge::Current;
                addresses
            },
            projected_origins: SmallSet::Empty,
            readonly_source,
            cyclic: false,
            prior: wrapper.age == WrapperAge::Prior,
        }
    }

    fn resolution_frame<'a>(
        &self,
        row: &'a BindingRow,
        aliases_only: bool,
        readonly_source: Option<usize>,
    ) -> ResolutionFrame<'a> {
        let wrapper = row.wrapper;
        ResolutionFrame {
            wrapper,
            alternatives: row.alts.as_slice().iter(),
            result: ResolvedCells {
                addresses: {
                    let mut addresses = AddressSet::from_cells(
                        [],
                        Self::wrapper_coverage(
                            self.current_wrapper_init(wrapper),
                            Some(row.coverage),
                        ),
                    );
                    addresses.all_candidates_known = row.coverage == Coverage::Complete;
                    addresses.routes.clone_from(&row.routes);
                    addresses
                },
                projected_origins: SmallSet::Empty,
                readonly_source,
                cyclic: false,
                prior: wrapper.age == WrapperAge::Prior,
            },
            aliases_only,
        }
    }

    pub(super) fn binding_route_roots(&self, local: usize) -> RootSet {
        let mut roots = RootSet::empty(self.locals.len());
        let wrapper = WrapperId::current(local);
        if self.binding_row(wrapper).is_none() {
            return roots;
        }
        let mut pending = vec![wrapper];
        let mut seen = FxHashSet::default();
        while let Some(wrapper) = pending.pop() {
            if !seen.insert(wrapper) {
                continue;
            }
            let Some(row) = self.binding_row(wrapper) else {
                continue;
            };
            let isolated_retired_holder = row.coverage == Coverage::Complete
                && !row.alts.as_slice().is_empty()
                && row.alts.as_slice().iter().all(|alt| {
                    if let (BindingTarget::Projected(holder, _), Some(origin)) = (alt.target, alt.origin)
                        && origin.isolated
                    {
                        let table = self.layer.table.borrow();
                        matches!(table.base(holder), AllocBase::Old(site, _) | AllocBase::Departed(site)
                            if site as usize == origin.wrapper.local)
                    } else {
                        false
                    }
                });
            if !isolated_retired_holder {
                roots.union_with(&row.route_roots);
            }
            for alt in row.alts.as_slice() {
                if let BindingTarget::Link(source) = alt.target {
                    if source.age == WrapperAge::Current && source.local != local {
                        roots.insert(source.local);
                    }
                    pending.push(source);
                }
            }
        }
        roots
    }

    pub(in super::super) fn projected_cell_is_exact(&self, holder: AllocId, key: FieldKey) -> bool {
        matches!(key, FieldKey::Field(_) | FieldKey::Payload { .. }) && self.alloc_singular(holder)
    }

    pub(in super::super) fn has_captured_projection(&self, holder: AllocId, key: FieldKey) -> bool {
        !self.layer.edges.group(holder, key).is_empty()
            || !self
                .layer
                .contents(CellId::Projected(holder, key))
                .is_empty()
    }

    fn slot_roots(&self, wrapper: WrapperId) -> &RootSet {
        self.binding_row(wrapper)
            .and_then(|row| row.slot_roots.as_ref())
            .unwrap_or(&self.locals[wrapper.local].value_roots)
    }

    pub(in super::super) fn layer_is_empty(&self) -> bool {
        self.layer.is_empty()
    }

    pub(in super::super) fn use_indexed_recurring_blocks(&self, blocks: Rc<FxHashSet<BlockId>>) {
        self.layer.table.borrow_mut().cyclic_blocks = Some(RecurringBlocks::Shared(blocks));
    }

    #[cfg(test)]
    pub(in super::super) fn shares_test_recurring_index(
        &self,
        index: &Rc<FxHashSet<BlockId>>,
    ) -> bool {
        matches!(&self.layer.table.borrow().cyclic_blocks,
            Some(RecurringBlocks::Shared(blocks)) if Rc::ptr_eq(blocks, index))
    }

    /// Whether `block` lies on a CFG cycle, from the layout's successor lists.
    ///
    /// WHY: one construction in an acyclic block builds at most one allocation per call, so its
    ///      site id may take strong updates. A site in a loop names one allocation per iteration.
    pub(in super::super) fn site_id(
        &self,
        site: u32,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> AllocId {
        self.stable_site_id(AllocBase::Site(site), block, successors)
    }

    pub(in super::super) fn call_site_id(
        &self,
        order: OrderKey,
        error: bool,
        node: u32,
        summarized: bool,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> AllocId {
        let base = AllocBase::Call(order, error, node);
        if summarized {
            self.layer.table.borrow_mut().base_id(base, false)
        } else {
            self.stable_site_id(base, block, successors)
        }
    }

    pub(in super::super) fn opaque_call_id(
        &self,
        order: OrderKey,
        error: bool,
        scope: u32,
        ty: u32,
    ) -> AllocId {
        self.layer
            .table
            .borrow_mut()
            .base_id(AllocBase::OpaqueCall(order, error, scope, ty), false)
    }

    pub(in super::super) fn call_view_id(
        &self,
        order: OrderKey,
        error: bool,
        node: u32,
    ) -> AllocId {
        self.layer
            .table
            .borrow_mut()
            .base_id(AllocBase::CallView(order, error, node), false)
    }

    pub(in super::super) fn add_view_member(&mut self, view: AllocId, id: AllocId) {
        self.layer.sharing.edit_row(
            &self.layer.table.borrow(),
            SharingRow::Member(view, id),
            false,
            true,
        );
    }

    pub(in super::super) fn relate_views(&mut self, a: AllocId, b: AllocId, may_share: bool) {
        let (a, b) = ordered_pair(a, b);
        self.layer.sharing.edit_row(
            &self.layer.table.borrow(),
            SharingRow::Pair(a, b),
            may_share,
            true,
        );
    }

    pub(in super::super) fn link_view(&mut self, holder: AllocId, key: FieldKey, held: AllocId) {
        increment_frontend_counter(FrontendCounter::BorrowPublisherGroupRowsInspected);
        if self.layer.sharing.edit_row(
            &self.layer.table.borrow(),
            SharingRow::Projection(holder, key, held),
            false,
            true,
        ) {
            increment_frontend_counter(FrontendCounter::BorrowPublisherGroupRowsMoved);
        }
    }

    fn bind_slot_views(&mut self, local: WrapperId, views: &[AllocId]) {
        let table = self.layer.table.borrow();
        self.layer.sharing.clear_bindings(&table, local);
        for &view in views {
            self.layer
                .sharing
                .edit_row(&table, SharingRow::Binding(local, view), false, true);
        }
    }

    pub(in super::super) fn alloc_paths_may_share(
        &self,
        left: &[AllocId],
        left_views: &[AllocId],
        right: &[AllocId],
        right_views: &[AllocId],
    ) -> bool {
        let left = if left_views.is_empty() {
            left
        } else {
            left_views
        };
        let right = if right_views.is_empty() {
            right
        } else {
            right_views
        };
        left.iter()
            .any(|&a| right.iter().any(|&b| self.layer.views_may_share(a, b)))
    }

    pub(in super::super) fn visit_sharing_incidence(
        &self,
        id: AllocId,
        mut each: impl FnMut((AllocId, AllocId)) -> std::ops::ControlFlow<()>,
    ) -> std::ops::ControlFlow<()> {
        each((id, id))?;
        increment_frontend_counter(FrontendCounter::BorrowOperandSharingRows);
        for member in self.layer.sharing.members(id) {
            increment_frontend_counter(FrontendCounter::BorrowOperandSharingRows);
            each((member, member))?;
        }
        self.layer.sharing.pairs.visit_incidence(id, each)
    }

    // The call index adds paths to argument targets without enumerating the state's interner.
    pub(in super::super) fn visit_operand_path(
        &self,
        mut id: AllocId,
        mut add: impl FnMut(AllocId, AllocId) -> bool,
    ) {
        let table = self.layer.table.borrow();
        loop {
            increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
            let Some((parent, _)) = table.parent(id) else {
                break;
            };
            if !add(parent, id) {
                break;
            }
            id = parent;
        }
    }

    pub(in super::super) fn operand_lazy_child(&self, child: AllocId) -> bool {
        self.layer
            .covered_parent(&self.layer.table.borrow(), child, false, true)
            .is_some()
    }

    pub(in super::super) fn visit_operand_reach(
        &self,
        id: AllocId,
        reverse: bool,
        mut visit: impl FnMut(AllocId) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let table = self.layer.table.borrow();
        if reverse {
            if let Some(parent) = self.layer.covered_parent(&table, id, false, true) {
                visit(parent)?;
            }
            increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
            for holder in self.layer.edges.incoming_holders(id) {
                increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
                visit(holder)?;
            }
            ControlFlow::Continue(())
        } else {
            self.visit_forward_links(id, false, true, |next, explicit| {
                if explicit
                    || self
                        .layer
                        .covered_parent(&table, next, false, true)
                        .is_some()
                {
                    visit(next)?;
                }
                ControlFlow::Continue(())
            })
        }
    }

    fn visit_forward_links(
        &self,
        id: AllocId,
        writable_edges: bool,
        count_incidence: bool,
        mut push: impl FnMut(AllocId, bool) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if count_incidence {
            increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
        }
        for edge in self.layer.edges.for_holder(id) {
            if count_incidence {
                increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
            }
            if edge.held != AllocId::EMPTY && (!writable_edges || edge.writable) {
                push(edge.held, true)?;
            }
        }
        if count_incidence {
            increment_frontend_counter(FrontendCounter::BorrowOperandReachRows);
        }
        for child in self.layer.edges.children_of(id) {
            push(child, false)?;
        }
        ControlFlow::Continue(())
    }

    fn walk_explicit_allocations(
        &self,
        from: &[AllocId],
        writable_edges: bool,
        writable_paths: bool,
        mut visit: impl FnMut(&AllocTable, AllocId) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if from.is_empty() {
            return ControlFlow::Continue(());
        }
        let table = self.layer.table.borrow();
        for &root in from {
            visit(&table, root)?;
        }
        if self.layer.edges.is_empty() || !from.iter().any(|&id| self.layer.edges.has_outgoing(id))
        {
            return ControlFlow::Continue(());
        }
        // A holder can be visited through a lazy path before it is emitted as an explicit value.
        let mut seen: FxHashMap<_, _> = from.iter().map(|&id| (id, true)).collect();
        let mut pending = from.to_vec();
        while let Some(id) = pending.pop() {
            self.visit_forward_links(id, writable_edges, false, |next, explicit| {
                if !explicit
                    && self
                        .layer
                        .covered_parent(&table, next, writable_paths, false)
                        .is_none()
                {
                    return ControlFlow::Continue(());
                }
                let emitted = seen.entry(next).or_insert_with(|| {
                    pending.push(next);
                    false
                });
                if explicit && !std::mem::replace(emitted, true) {
                    visit(&table, next)?;
                }
                ControlFlow::Continue(())
            })?;
        }
        ControlFlow::Continue(())
    }

    #[cfg(test)]
    pub(in super::super) fn independent_test_state(&self) -> Self {
        let mut state = self.clone();
        state.layer.table = Rc::new(RefCell::new(self.layer.table.borrow().clone()));
        state
    }

    #[cfg(test)]
    pub(in super::super) fn canonical_test_allocations(
        &self,
        vocabulary: &mut BTreeMap<String, AllocId>,
    ) -> impl Fn(AllocId) -> AllocId + use<> {
        let table = self.layer.table.borrow();
        let mut keys = Vec::<String>::with_capacity(table.infos.len());
        let mut ids = Vec::with_capacity(table.infos.len());
        for info in &table.infos {
            let key_of = |id: AllocId| {
                if id == AllocId::EMPTY {
                    "<empty>"
                } else {
                    keys[id.0 as usize].as_str()
                }
            };
            let name = match info.key {
                AllocKey::Base(base) => format!("{base:?}"),
                AllocKey::Child(parent, selector) => {
                    format!("child({},{selector:?})", key_of(parent))
                }
                AllocKey::Copied(graph, source) => {
                    format!("copy({},{})", key_of(graph), key_of(source))
                }
                AllocKey::CopiedOccurrence(graph, source) => {
                    format!("occurrence({},{})", key_of(graph), key_of(source))
                }
                AllocKey::CopyPair(graph, a, b) => {
                    let (a, b) = (key_of(a), key_of(b));
                    format!("pair({},{},{})", key_of(graph), a.min(b), a.max(b))
                }
            };
            let next = AllocId(vocabulary.len() as u32);
            ids.push(*vocabulary.entry(name.clone()).or_insert(next));
            keys.push(name);
        }
        move |id| {
            if id == AllocId::EMPTY {
                id
            } else {
                ids[id.0 as usize]
            }
        }
    }

    #[cfg(test)]
    pub(in super::super) fn canonical_test_edge_rows(
        &self,
        map: impl Fn(AllocId) -> AllocId,
    ) -> Vec<String> {
        let layer = &self.layer;
        let mut rows = vec![format!("locals {:?}", self.locals)];
        for edge in layer.edges.iter() {
            rows.push(format!(
                "edge {:?} {:?} {:?} {} {}",
                map(edge.holder),
                edge.key,
                map(edge.held),
                edge.writable,
                edge.single
            ));
        }
        for &(slot, id, readonly) in layer.bound.values().flatten() {
            rows.push(format!("bound {slot:?} {:?} {readonly}", map(id)));
        }
        for (&cell, values) in &layer.contents {
            let mut values = values.clone();
            remap_value_alternatives(&mut values, &map);
            rows.push(format!(
                "cell {:?} {values:?}",
                cell.remap(|slot| slot, &map)
            ));
        }
        for row in &layer.wrappers {
            let mut row = row.clone();
            row.alts.remap(|mut alt| {
                if let BindingTarget::Projected(holder, key) = alt.target {
                    alt.target = BindingTarget::Projected(map(holder), key);
                }
                alt
            });
            for (cell, deps) in &mut row.routes {
                *cell = cell.remap(|slot| slot, &map);
                super::remap_dependencies(deps, &map);
            }
            row.routes.sort_unstable();
            rows.push(format!("wrapper {row:?}"));
        }
        for entry in layer.observations.values() {
            let mut entry = entry.clone();
            for value in &mut entry.values {
                value.id = map(value.id);
            }
            for view in &mut entry.views {
                *view = map(*view);
            }
            remap_value_alternatives(&mut entry.alternatives, &map);
            normalize_observation(&mut entry);
            rows.push(format!("observation {entry:?}"));
        }
        for &(local, id) in &layer.detached {
            rows.push(format!("detached {local} {:?}", map(id)));
        }
        for &(view, member) in &layer.sharing.members {
            rows.push(format!("member {:?} {:?}", map(view), map(member)));
        }
        for (&(holder, key), children) in &layer.sharing.projections {
            for &child in children {
                rows.push(format!(
                    "projection {:?} {key:?} {:?}",
                    map(holder),
                    map(child)
                ));
            }
        }
        for &(slot, view) in &layer.sharing.bindings {
            rows.push(format!("view {slot:?} {:?}", map(view)));
        }
        for &(a, b, share) in layer.sharing.pairs.iter() {
            rows.push(format!("pair {:?} {share}", ordered_pair(map(a), map(b))));
        }
        rows.sort_unstable();
        rows
    }

    #[cfg(test)]
    pub(in super::super) fn assert_allocation_incidence(&self) {
        let table = self.layer.table.borrow();
        let mut children = BTreeSet::new();
        let mut incoming = BTreeSet::new();
        let mut bases = FxHashMap::<_, BTreeSet<_>>::default();
        for edge in self.layer.edges.iter() {
            let group = (edge.holder, edge.key);
            bases
                .entry(table.base(edge.holder))
                .or_default()
                .insert(group);
            if edge.held != AllocId::EMPTY {
                incoming.insert((edge.held, edge.holder, edge.key));
                bases
                    .entry(table.base(edge.held))
                    .or_default()
                    .insert(group);
            }
            let mut id = edge.holder;
            while let Some((parent, _)) = table.parent(id) {
                if !children.insert((parent, id)) {
                    break;
                }
                id = parent;
            }
        }
        assert_eq!(self.layer.edges.children, children);
        assert_eq!(self.layer.edges.incoming, incoming);
        assert_eq!(self.layer.edges.by_base, bases);
        let pairs = &self.layer.sharing.pairs;
        let expected_pairs: BTreeSet<_> = pairs
            .iter()
            .filter_map(|&(left, right, shares)| (shares && left != right).then_some((right, left)))
            .collect();
        assert_eq!(pairs.positive_by_right, expected_pairs);
        assert_eq!(
            self.layer.sharing.member_views,
            self.layer
                .sharing
                .members
                .iter()
                .map(|&(view, id)| (id, view))
                .collect(),
        );
        let mut sharing = BTreeMap::<_, BTreeSet<_>>::new();
        let mut sharing_row = |row, ids: &[AllocId]| {
            for &id in ids {
                if id != AllocId::EMPTY {
                    sharing.entry(table.base(id)).or_default().insert(row);
                }
            }
        };
        for &(view, member) in &self.layer.sharing.members {
            sharing_row(SharingRow::Member(view, member), &[view, member]);
        }
        for (&(holder, key), children) in &self.layer.sharing.projections {
            for &child in children {
                sharing_row(SharingRow::Projection(holder, key, child), &[holder, child]);
            }
        }
        for &(slot, view) in &self.layer.sharing.bindings {
            sharing_row(SharingRow::Binding(slot, view), &[view]);
        }
        for &(a, b, _) in pairs.iter() {
            sharing_row(SharingRow::Pair(a, b), &[a, b]);
        }
        assert_eq!(self.layer.sharing.by_base, sharing);
        let detached = self
            .layer
            .detached
            .iter()
            .filter_map(|&(owner, holder)| {
                (holder != AllocId::EMPTY).then(|| (table.base(holder), owner, holder))
            })
            .collect();
        assert_eq!(self.layer.detached_by_base, detached);
        let expected_routes: BTreeMap<_, _> = self
            .layer
            .observations
            .keys()
            .map(|&local| {
                let address = self.resolve_cells(local);
                let mut routes = BTreeSet::new();
                if address.coverage == Coverage::Complete {
                    for &cell in address.cells() {
                        routes.insert(match cell {
                            CellId::Slot(slot) => ObservationRoute::Slot(slot),
                            CellId::Projected(_, selector) => ObservationRoute::Selector(selector),
                        });
                    }
                } else {
                    routes.insert(ObservationRoute::Incomplete);
                }
                (local, routes)
            })
            .collect();
        assert_eq!(self.layer.observation_routes, expected_routes);
        let expected_reverse = expected_routes
            .iter()
            .flat_map(|(&local, routes)| routes.iter().map(move |&route| (route, local)))
            .collect();
        assert_eq!(self.layer.observations_by_route, expected_reverse);

        let mut observations = BTreeSet::new();
        for (&local, row) in &self.layer.observations {
            let mut add = |id| {
                if id != AllocId::EMPTY {
                    observations.insert((table.base(id), local));
                }
            };
            for value in &row.values {
                add(value.id);
            }
            for &id in &row.views {
                add(id);
            }
            for alternative in &row.alternatives {
                if let ValueKind::Allocation(value) = alternative.kind {
                    add(value.id);
                }
                for dep in &alternative.deps {
                    for &id in dep.entries.as_slice() {
                        add(id);
                    }
                }
            }
        }
        assert_eq!(self.layer.observations_by_base, observations);
        let bounds = self
            .layer
            .bound
            .values()
            .flatten()
            .filter_map(|&(slot, id, _)| (id != AllocId::EMPTY).then(|| (table.base(id), slot)))
            .collect();
        assert_eq!(self.layer.bound_by_base, bounds);
        let mut wrappers = BTreeSet::new();
        for row in &self.layer.wrappers {
            let mut add = |id| {
                if id != AllocId::EMPTY {
                    wrappers.insert((table.base(id), row.wrapper));
                }
            };
            for alt in row.alts.as_slice() {
                if let BindingTarget::Projected(holder, _) = alt.target {
                    add(holder);
                }
            }
            for (cell, deps) in &row.routes {
                if let CellId::Projected(holder, _) = *cell {
                    add(holder);
                }
                for dep in deps {
                    for &id in dep.entries.as_slice() {
                        add(id);
                    }
                }
            }
        }
        assert_eq!(self.layer.wrappers_by_base, wrappers);
        let wrapper_rows = self.layer.wrappers.iter();
        let mut expected_users = BTreeSet::new();
        let mut expected_slots = BTreeSet::new();
        for row in wrapper_rows {
            for alt in row.alts.as_slice() {
                let link = match alt.target {
                    BindingTarget::Link(target) => Some(target),
                    _ => None,
                };
                for target in [
                    link,
                    alt.origin.map(|origin| origin.wrapper),
                    alt.origin.and_then(|origin| origin.owner),
                ]
                .into_iter()
                .flatten()
                {
                    expected_users.insert((target, row.wrapper));
                }
            }
            if row.wrapper.age == WrapperAge::Current
                && row.has_slot()
                && let Some(roots) = &row.slot_roots
            {
                expected_slots.extend(roots.iter_ones().map(|root| (root, row.wrapper.local)));
            }
        }
        assert_eq!(expected_users, self.layer.wrapper_users);
        assert_eq!(expected_slots, self.layer.slot_root_users);
        assert_eq!(
            self.layer.route_rows,
            self.layer
                .wrappers
                .iter()
                .filter(|row| !row.routes.is_empty())
                .count()
        );
        let mut cells = FxHashMap::<_, BTreeSet<_>>::default();
        for (&cell, values) in &self.layer.contents {
            let holder = match cell {
                CellId::Projected(holder, _) => Some(holder),
                CellId::Slot(_) => None,
            };
            let stored = values.iter().filter_map(|value| match value.kind {
                ValueKind::Allocation(stored) => Some(stored.id),
                _ => None,
            });
            let dependencies = values
                .iter()
                .flat_map(|value| &value.deps)
                .flat_map(|dependency| dependency.entries.as_slice().iter().copied());
            for id in holder
                .into_iter()
                .chain(stored)
                .chain(dependencies)
                .filter(|&id| id != AllocId::EMPTY)
            {
                cells.entry(table.base(id)).or_default().insert(cell);
            }
        }
        assert_eq!(cells, self.layer.cells_by_base);
        let value_roots = self
            .locals
            .iter()
            .enumerate()
            .flat_map(|(local, row)| row.value_roots.iter_ones().map(move |root| (root, local)))
            .collect::<BTreeSet<_>>();
        let derivation = self
            .locals
            .iter()
            .enumerate()
            .flat_map(|(local, row)| {
                row.derived_from_roots
                    .iter_ones()
                    .map(move |root| (root, local))
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(value_roots, self.value_root_users);
        assert_eq!(derivation, self.derivation_users);
    }

    /// The pre-index traversal is an independent oracle over authoritative rows.
    #[cfg(test)]
    pub(in super::super) fn full_scan_typed_reach_for_test(
        &self,
        from: &[AllocId],
        target: AllocId,
        writable_only: bool,
    ) -> bool {
        let table = self.layer.table.borrow();
        let covers = |from, target| {
            self.layer
                .covers_through(&table, from, target, writable_only)
        };
        let mut seen = FxHashSet::default();
        let mut pending = from.to_vec();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if covers(id, target) {
                return true;
            }
            for edge in self.layer.edges.iter() {
                if edge.held != AllocId::EMPTY
                    && (!writable_only || edge.writable)
                    && covers(id, edge.holder)
                {
                    pending.push(edge.held);
                }
            }
        }
        false
    }

    pub(in super::super) fn block_may_repeat(
        &self,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> bool {
        self.layer
            .table
            .borrow_mut()
            .block_may_repeat(block, successors)
    }

    fn stable_site_id(
        &self,
        base: AllocBase,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> AllocId {
        let mut table = self.layer.table.borrow_mut();
        let singular = !table.block_may_repeat(block, successors);
        table.base_id(base, singular)
    }

    pub(in super::super) fn alloc_singular(&self, id: AllocId) -> bool {
        self.layer.table.borrow().singular(id)
    }

    /// The local that supplied this allocation, even after its binding was retired.
    pub(in super::super) fn alloc_origin_local(&self, id: AllocId) -> Option<usize> {
        base_local(self.layer.table.borrow().base(id))
    }

    pub(in super::super) fn is_current_allocation_of(&self, id: AllocId, local: usize) -> bool {
        id != AllocId::EMPTY && self.layer.table.borrow().base(id) == AllocBase::Root(local as u32)
    }

    /// Allocations a binding holds, and whether only layer facts name them.
    pub(in super::super) fn local_alloc_ids(&self, local_index: usize) -> (Vec<AllocId>, bool) {
        let resolved = self.resolve_local_allocs(local_index);
        (resolved.ids, resolved.layer_only)
    }

    #[cfg(test)]
    pub(in super::super) fn alloc_binding_readonly(&self, local: usize) -> bool {
        self.resolve_local_allocs(local).readonly
    }

    fn slot_values(&self, slot: WrapperId) -> impl Iterator<Item = StoredValue> + '_ {
        let bound = self.layer.bound_of(slot);
        let roots = self.slot_roots(slot);
        let implicit = (bound.is_empty() && roots.is_empty()).then_some(slot.local);
        let override_values = bound.iter().filter_map(|&(_, id, readonly)| {
            (id != AllocId::EMPTY).then_some(StoredValue {
                id,
                writable: !readonly,
            })
        });
        let lazy_values = bound
            .is_empty()
            .then_some(roots)
            .into_iter()
            .flat_map(RootSet::iter_ones)
            .chain(implicit)
            .map(move |root| {
                let mut table = self.layer.table.borrow_mut();
                let id = if root == slot.local && slot.age == WrapperAge::Prior {
                    table.base_id(AllocBase::Departed(root as u32), false)
                } else {
                    table.root(root)
                };
                StoredValue { id, writable: true }
            });
        override_values.chain(lazy_values)
    }

    /// A whole link loads its terminal cell; a projected location loads the retained holder.
    pub(in super::super) fn resolve_local_allocs(&self, local: usize) -> ResolvedPlace {
        let addresses = self.resolve_cells(local);
        self.resolve_local_allocs_from_cells(local, &addresses)
    }

    pub(in super::super) fn capture_jump_value(
        &self,
        local: usize,
        scalar: bool,
        force_allocation: bool,
    ) -> Option<CapturedValue> {
        let has_payload = self.has_value_alternatives();
        let has_binding = self.has_layer_binding(local);
        if !has_payload && !has_binding && !force_allocation {
            return None;
        }
        let addresses = self.resolve_cells(local);
        if scalar && has_payload {
            let alternatives = self.scalar_cell_alternatives(Some(local), &addresses);
            if !alternatives.is_empty() {
                return Some(
                    CapturedValue {
                        alternatives,
                        ..CapturedValue::default()
                    }
                    .into_scalar(),
                );
            }
        }
        if has_binding || force_allocation || !scalar {
            let resolved = self.resolve_local_allocs_from_cells(local, &addresses);
            if !has_binding && !force_allocation && resolved.alternatives.is_empty() {
                return None;
            }
            let readonly = resolved.readonly && (has_binding || !force_allocation);
            Some(CapturedValue {
                values: resolved
                    .ids
                    .into_iter()
                    .map(|id| StoredValue {
                        id,
                        writable: !readonly,
                    })
                    .collect(),
                views: if force_allocation && !has_binding {
                    Vec::new()
                } else {
                    resolved.views
                },
                alternatives: resolved.alternatives,
            })
        } else {
            None
        }
    }

    fn resolve_local_allocs_from_cells(
        &self,
        local: usize,
        addresses: &AddressSet,
    ) -> ResolvedPlace {
        if let Some(entry) = self.layer.observation(local) {
            return ResolvedPlace {
                ids: entry.values.iter().map(|value| value.id).collect(),
                views: entry.views.clone(),
                layer_only: true,
                readonly: entry.readonly || entry.values.iter().any(|value| !value.writable),
                readonly_source: None,
                coverage: Coverage::Complete,
                may_be_absent: false,
                alternatives: observed_alternatives(entry, addresses),
            };
        }
        self.resolve_captured_allocs(addresses, Some(local))
    }

    pub(in super::super) fn resolve_captured_allocs(
        &self,
        addresses: &AddressSet,
        actor: Option<usize>,
    ) -> ResolvedPlace {
        let mut result = ResolvedPlace {
            ids: Vec::new(),
            views: Vec::new(),
            layer_only: false,
            readonly: false,
            readonly_source: None,
            coverage: addresses.may_coverage(),
            may_be_absent: addresses.coverage == Coverage::Partial
                && addresses.may_coverage() == Coverage::Complete,
            alternatives: Vec::new(),
        };
        for &cell in addresses.cells() {
            let mut value = match cell {
                CellId::Slot(slot) => {
                    let (mut ids, mut readonly) = (Vec::new(), false);
                    for value in self.slot_values(slot) {
                        ids.push(value.id);
                        readonly |= !value.writable;
                    }
                    ResolvedPlace {
                        ids,
                        views: self.layer.sharing.bindings(slot).collect(),
                        layer_only: !self.layer.bound_of(slot).is_empty()
                            || slot.age == WrapperAge::Prior,
                        readonly,
                        readonly_source: None,
                        coverage: Coverage::Complete,
                        may_be_absent: false,
                        alternatives: self.layer.contents(cell).to_vec(),
                    }
                }
                CellId::Projected(holder, key) => {
                    let mut projected = ResolvedPlace {
                        ids: vec![holder],
                        views: Vec::new(),
                        layer_only: true,
                        readonly: false,
                        readonly_source: None,
                        coverage: Coverage::Complete,
                        may_be_absent: false,
                        alternatives: Vec::new(),
                    };
                    for (target, deps) in &addresses.routes {
                        if *target == cell {
                            projected.alternatives.push(ValueAlt {
                                kind: ValueKind::Allocation(StoredValue {
                                    id: holder,
                                    writable: true,
                                }),
                                deps: deps.clone(),
                            });
                        }
                    }
                    self.step_resolved(&mut projected, key, &[]);
                    projected
                }
            };
            if !value.alternatives.is_empty() || !result.alternatives.is_empty() {
                if result.alternatives.is_empty() && !result.ids.is_empty() {
                    result.alternatives =
                        compact_alternatives(result.ids.iter().map(|&id| StoredValue {
                            id,
                            writable: !result.readonly,
                        }));
                }
                if value.alternatives.is_empty() {
                    value.alternatives =
                        compact_alternatives(value.ids.iter().map(|&id| StoredValue {
                            id,
                            writable: !value.readonly,
                        }));
                }
                merge_alternatives(&mut result.alternatives, value.alternatives);
            }
            if result.views.is_empty() && result.ids.is_empty() {
                result.views = value.views;
            } else if !result.views.is_empty() || !value.views.is_empty() {
                if result.views.is_empty() {
                    result.views.extend_from_slice(&result.ids);
                }
                if value.views.is_empty() {
                    result.views.extend_from_slice(&value.ids);
                } else {
                    result.views.append(&mut value.views);
                }
            }
            if result.ids.is_empty() {
                result.ids = value.ids;
            } else {
                result.ids.append(&mut value.ids);
            }
            result.layer_only |= value.layer_only;
            result.readonly |= value.readonly;
            result.readonly_source = result.readonly_source.or(value.readonly_source);
            if value.coverage == Coverage::Partial {
                result.coverage = Coverage::Partial;
            }
            result.may_be_absent |= value.may_be_absent;
        }
        if addresses.may_coverage() == Coverage::Partial
            && let Some(local) = actor
            && self.locals[local].init != Init::No
        {
            let mut table = self.layer.table.borrow_mut();
            result.ids.extend(
                self.locals[local]
                    .value_roots
                    .iter_ones()
                    .map(|root| table.root(root)),
            );
        }
        if !result.alternatives.is_empty() && result.coverage == Coverage::Partial {
            merge_alternatives(
                &mut result.alternatives,
                [ValueAlt::plain(ValueKind::Unknown)],
            );
        }
        result.ids.sort_unstable();
        result.ids.dedup();
        result.views.sort_unstable();
        result.views.dedup();
        result
    }

    /// Resolve wrapper links and consume the contents of each prefix, never the final cell.
    pub(in super::super) fn resolve_place_path(
        &self,
        local: usize,
        local_mutable: &[bool],
        keys: impl IntoIterator<Item = FieldKey>,
    ) -> PlaceResolution {
        let binding = self.resolve_wrapper(WrapperId::current(local), false, local_mutable);
        let mut address = binding.addresses;
        let base_owner = match address.definite() {
            Some(CellId::Slot(slot)) => Some(slot),
            _ => None,
        };
        let mut traversal = SmallSet::Empty;
        let mut traversal_coverage = Coverage::Complete;
        let mut terminal_projection = None;
        for key in keys {
            for &cell in address.cells() {
                traversal.insert(cell);
            }
            if address.may_coverage() == Coverage::Partial {
                traversal_coverage = Coverage::Partial;
            }
            let mut holders =
                self.load_path_cell(local, &address, terminal_projection, local_mutable);
            self.mark_alloc_provenance_readonly(local_mutable, &mut holders);
            address = self.project_addresses(&holders, key);
            terminal_projection = Some((holders, key));
        }
        let holders = terminal_projection.as_ref().map(|(holders, _)| holders);
        let permission = ResolvedPermission {
            wrapper_readonly_source: binding.readonly_source,
            holder_readonly: holders.is_some_and(|holders| holders.readonly),
            holder_readonly_source: holders.and_then(|holders| holders.readonly_source),
        };
        PlaceResolution {
            address,
            traversal,
            traversal_coverage,
            content: None,
            permission,
            route: AccessRoute {
                local,
                projected: terminal_projection.is_some(),
            },
            projected_origins: binding.projected_origins,
            base_owner,
            terminal_projection,
        }
    }

    fn project_addresses(&self, holders: &ResolvedPlace, key: FieldKey) -> AddressSet {
        let mut address = AddressSet::from_cells(
            holders
                .ids
                .iter()
                .map(|&holder| CellId::Projected(holder, key)),
            if holders.ids.is_empty() {
                Coverage::Partial
            } else {
                holders.holder_coverage()
            },
        );
        if matches!(holders.ids.as_slice(), [holder] if self.alloc_singular(*holder)) {
            address = address.with_singleton_proof();
        }
        address.all_candidates_known = holders.coverage == Coverage::Complete;
        if holders.alternatives.iter().any(|alt| !alt.deps.is_empty()) {
            for alt in &holders.alternatives {
                match alt.kind {
                    ValueKind::Allocation(value) => {
                        address
                            .routes
                            .push((CellId::Projected(value.id, key), alt.deps.clone()));
                    }
                    ValueKind::Unknown if !alt.deps.is_empty() => {
                        address.routes.extend(
                            holders
                                .ids
                                .iter()
                                .map(|&holder| (CellId::Projected(holder, key), alt.deps.clone())),
                        );
                    }
                    _ => {}
                }
            }
            address.routes.sort_unstable();
            address.routes.dedup();
        }
        address
    }

    pub(in super::super) fn capture_scalar_projection(
        &self,
        holders: &ResolvedPlace,
        key: FieldKey,
    ) -> CapturedValue {
        if !self.has_value_alternatives() && holders.alternatives.is_empty() {
            return CapturedValue::default();
        }
        let address = self.project_addresses(holders, key);
        let alternatives = self.scalar_cell_alternatives(None, &address);
        CapturedValue {
            alternatives,
            ..CapturedValue::default()
        }
        .into_scalar()
    }

    /// Scalar observations need the cell, but cannot contribute an allocation alternative.
    pub(in super::super) fn load_place_content(
        &self,
        place: &mut PlaceResolution,
        local_mutable: &[bool],
        scalar: bool,
    ) {
        if place.content.is_none() {
            place.content = Some(if scalar {
                let mut result = ResolvedPlace::from_values(&[], Vec::new());
                if self.has_value_alternatives() || !place.address.retained_routes().is_empty() {
                    let local = (!place.route.projected).then_some(place.route.local);
                    result.alternatives = self.scalar_cell_alternatives(local, &place.address);
                }
                place.terminal_projection = None;
                result
            } else {
                self.load_path_cell(
                    place.route.local,
                    &place.address,
                    place.terminal_projection.take(),
                    local_mutable,
                )
            });
        }
    }

    fn scalar_cell_alternatives(
        &self,
        local: Option<usize>,
        address: &AddressSet,
    ) -> ValueAlternatives {
        if let Some(entry) = local.and_then(|local| self.layer.observation(local)) {
            return observed_alternatives(entry, address);
        }
        let mut alternatives = Vec::new();
        let mut plain = false;
        let mut absent = false;
        for &cell in address.cells() {
            let group = self.layer.contents(cell);
            let implicit_kind = match cell {
                CellId::Projected(holder, key)
                    if self
                        .layer
                        .edges
                        .group(holder, key)
                        .iter()
                        .any(|edge| edge.held == AllocId::EMPTY) =>
                {
                    ValueKind::Absent
                }
                _ => ValueKind::Scalar,
            };
            let before = alternatives.len();
            for (target, deps) in address.retained_routes() {
                if *target != cell {
                    continue;
                }
                if group.is_empty() {
                    alternatives.push(ValueAlt {
                        kind: implicit_kind,
                        deps: deps.clone(),
                    });
                } else {
                    alternatives.extend(
                        group
                            .iter()
                            .cloned()
                            .map(|alt| inherit_dependencies(alt, deps)),
                    );
                }
            }
            if alternatives.len() == before {
                if group.is_empty() {
                    plain |= implicit_kind == ValueKind::Scalar;
                    absent |= implicit_kind == ValueKind::Absent;
                }
                alternatives.extend_from_slice(group);
            }
        }
        if !alternatives.is_empty() {
            if plain {
                alternatives.push(ValueAlt::plain(ValueKind::Scalar));
            }
            if absent {
                alternatives.push(ValueAlt::plain(ValueKind::Absent));
            }
            if address.coverage == Coverage::Partial {
                alternatives.push(ValueAlt::plain(ValueKind::Unknown));
            }
            alternatives.sort_unstable();
            alternatives.dedup();
        }
        alternatives
    }

    fn load_path_cell(
        &self,
        local: usize,
        address: &AddressSet,
        projection: Option<(ResolvedPlace, FieldKey)>,
        local_mutable: &[bool],
    ) -> ResolvedPlace {
        if let Some((mut holders, key)) = projection {
            self.step_resolved(&mut holders, key, local_mutable);
            holders
        } else {
            self.resolve_local_allocs_from_cells(local, address)
        }
    }

    /// The path into the value and the value's own origin both constrain mutation.
    pub(in super::super) fn mark_alloc_provenance_readonly(
        &self,
        local_mutable: &[bool],
        resolved: &mut ResolvedPlace,
    ) {
        if resolved.readonly_source.is_none()
            && let Some(origin) = resolved
                .ids
                .iter()
                .filter_map(|&id| self.alloc_origin_local(id))
                .find(|&origin| !local_mutable[origin])
        {
            resolved.readonly = true;
            resolved.readonly_source = Some(origin);
        }
    }

    #[cfg(test)]
    pub(in super::super) fn resolve_alloc_path(
        &self,
        local: usize,
        local_mutable: &[bool],
        keys: impl IntoIterator<Item = FieldKey>,
    ) -> ResolvedPlace {
        let mut result = self.resolve_place_path(local, local_mutable, keys);
        self.load_place_content(&mut result, local_mutable, false);
        result.content.expect("allocation path requests content")
    }

    /// Advance `resolved` through one field.
    pub(in super::super) fn step_resolved(
        &self,
        resolved: &mut ResolvedPlace,
        key: FieldKey,
        local_mutable: &[bool],
    ) {
        let mut table = self.layer.table.borrow_mut();
        if !resolved.alternatives.is_empty()
            || !self.layer.contents.is_empty()
                && resolved.ids.iter().any(|&holder| {
                    !self
                        .layer
                        .contents(CellId::Projected(holder, key))
                        .is_empty()
                })
        {
            let mut alternatives = Vec::new();
            for &holder in &resolved.ids {
                let group = self.layer.contents(CellId::Projected(holder, key));
                let parents = &resolved.alternatives;
                if !group.is_empty() {
                    append_projected_alternatives(
                        &mut alternatives,
                        parents,
                        holder,
                        group.iter().cloned(),
                    );
                } else {
                    let edges = self.layer.edges.group(holder, key);
                    if edges.is_empty() {
                        let child = ValueAlt::plain(ValueKind::Allocation(StoredValue {
                            id: table.child(holder, key),
                            writable: true,
                        }));
                        append_projected_alternatives(
                            &mut alternatives,
                            parents,
                            holder,
                            std::iter::once(child),
                        );
                    } else {
                        append_projected_alternatives(
                            &mut alternatives,
                            parents,
                            holder,
                            edges.iter().map(Edge::value_alternative),
                        );
                    }
                }
            }
            alternatives.extend(
                resolved
                    .alternatives
                    .iter()
                    .filter(|alt| matches!(alt.kind, ValueKind::Unknown | ValueKind::Absent))
                    .cloned(),
            );
            alternatives.sort_unstable();
            alternatives.dedup();
            resolved.alternatives = alternatives;
        }
        let views = if self.layer.sharing.is_empty() {
            Vec::new()
        } else {
            self.layer
                .project_views(&mut table, &resolved.ids, &resolved.views, key)
        };
        let mut next = Vec::with_capacity(resolved.ids.len());
        for holder in resolved.ids.drain(..) {
            let group = self.layer.edges.group(holder, key);
            if group.is_empty() {
                next.push(table.child(holder, key));
                continue;
            }
            resolved.layer_only = true;
            for edge in group {
                if edge.held == AllocId::EMPTY {
                    resolved.may_be_absent = true;
                    continue;
                }
                if !edge.writable && !resolved.readonly {
                    resolved.readonly = true;
                    resolved.readonly_source = base_local(table.base(edge.held))
                        .filter(|local| !local_mutable.get(*local).copied().unwrap_or(true));
                }
                next.push(edge.held);
            }
        }
        next.sort_unstable();
        next.dedup();
        resolved.ids = next;
        resolved.views = views;
    }

    /// Record that `local_index` holds `ids`, replacing any earlier override.
    #[cfg(test)]
    pub(in super::super) fn bind_alloc_ids(
        &mut self,
        local_index: usize,
        ids: &[AllocId],
        readonly: bool,
    ) {
        let slot = WrapperId::current(local_index);
        self.clear_slot_payload(slot);
        self.layer.clear_detachments(local_index);
        if ids.is_empty() {
            self.layer
                .set_bound(slot, vec![(slot, AllocId::EMPTY, readonly)]);
        } else {
            self.layer
                .extend_bound(slot, ids.iter().map(|&id| (slot, id, readonly)));
        }
    }

    pub(in super::super) fn has_layer_binding(&self, local: usize) -> bool {
        let wrapper = WrapperId::current(local);
        self.binding_row(wrapper).is_some()
            || !self.layer.bound_of(wrapper).is_empty()
            || self.layer.sharing.bindings(wrapper).next().is_some()
    }

    pub(in super::super) fn has_value_alternatives(&self) -> bool {
        !self.layer.contents.is_empty() || self.layer.route_rows != 0
    }

    fn clear_slot_payload(&mut self, slot: WrapperId) {
        self.clear_slot_allocation(slot);
        if !self.layer.contents.is_empty() {
            self.layer.set_contents(CellId::Slot(slot), Vec::new());
        }
    }

    fn clear_slot_allocation(&mut self, slot: WrapperId) {
        self.layer.set_bound(slot, Vec::new());
        if !self.layer.sharing.bindings.is_empty() {
            self.layer
                .sharing
                .clear_bindings(&self.layer.table.borrow(), slot);
        }
    }

    pub(in super::super) fn detached_holders(
        &self,
        local: usize,
    ) -> impl Iterator<Item = AllocId> + '_ {
        let local = local as u32;
        self.layer
            .detached
            .range((local, AllocId::MIN)..=(local, AllocId::MAX))
            .map(|&(_, holder)| holder)
    }

    pub(in super::super) fn bind_detached_holders(
        &mut self,
        local: usize,
        holders: impl IntoIterator<Item = AllocId>,
    ) {
        self.layer.clear_detachments(local);
        let table = self.layer.table.borrow();
        for holder in holders {
            AllocationLayer::edit_detachment(
                &mut self.layer.detached,
                &mut self.layer.detached_by_base,
                &table,
                (local as u32, holder),
                true,
            );
        }
    }

    pub(in super::super) fn invalidate_detachments(&mut self) {
        self.layer.detached.clear();
        self.layer.detached_by_base.clear();
    }

    pub(in super::super) fn detached_from(&self, local: usize, holders: &[AllocId]) -> bool {
        !holders.is_empty()
            && holders
                .iter()
                .all(|&holder| self.layer.detached.contains(&(local as u32, holder)))
    }

    fn graphs_overlap(&self, left: &[AllocId], right: &[AllocId]) -> bool {
        let children = self.alloc_closure(right, false);
        self.alloc_reaches_where(left, false, |table, current| {
            right.iter().chain(&children).any(|&target| {
                self.layer.covers(table, current, target)
                    || self.layer.covers(table, target, current)
            })
        })
    }

    fn proves_detached(&self, (local, holder): (u32, AllocId)) -> bool {
        self.layer.detached.contains(&(local, holder))
            || !self.graphs_overlap(&[holder], &self.local_alloc_ids(local as usize).0)
    }

    /// Install a new occurrence without changing a wrapper still reached by another name.
    pub(in super::super) fn rebind_local(
        &mut self,
        local_index: usize,
        new_state: LocalState,
        order: OrderKey,
    ) -> AllocationRetirement {
        let retirement = if self.layer.is_empty() || self.can_rebind_uninitialised(local_index) {
            AllocationRetirement {
                local: local_index,
                order,
                wrapper: true,
            }
        } else {
            self.retire_binding_allocation(local_index, order)
        };
        self.update_local_state(local_index, new_state);
        retirement
    }

    /// Publish a binding after its incoming allocation facts have been captured.
    pub(in super::super) fn rebind_local_after_capture(
        &mut self,
        local_index: usize,
        new_state: LocalState,
        order: OrderKey,
        pristine_definition: Option<usize>,
        eager_retirement: bool,
    ) -> Option<AllocationRetirement> {
        let retirement = if pristine_definition == Some(local_index)
            && self.can_rebind_uninitialised(local_index)
        {
            None
        } else if eager_retirement {
            Some(self.retire_binding_allocation(local_index, order))
        } else {
            // Even an empty layer can need a token to remap a captured lazy child.
            return Some(self.rebind_local(local_index, new_state, order));
        };
        self.update_local_state(local_index, new_state);
        retirement
    }

    /// Cheap exception check for a definition whose HIR prefix proves no earlier references.
    pub(in super::super) fn is_virgin_definition(&self, local: usize) -> bool {
        let current = WrapperId::current(local);
        self.locals[local].init == Init::No
            && self.binding_row(current).is_none()
            && self.binding_row(current.retired(|_| true)).is_none()
            && self.layer.bound_of(current).is_empty()
            && self.layer.contents(CellId::Slot(current)).is_empty()
            && self.layer.observation(local).is_none()
    }

    /// The HIR first-write proof excludes earlier coarse aliases, but captures can create
    /// new allocation facts while evaluating the RHS. Check those facts before publishing.
    pub(in super::super) fn can_publish_first_definition(&self, local: usize) -> bool {
        self.is_virgin_definition(local) && !self.has_explicit_root_references(local)
    }

    /// The caller must independently rule out coarse observers.
    pub(in super::super) fn has_pristine_alloc_binding(&self, local_index: usize) -> bool {
        if !self.is_virgin_definition(local_index) {
            return false;
        }
        let wrapper = WrapperId::current(local_index);
        if self.has_link_to(wrapper, false) {
            return false;
        }
        !self.has_explicit_root_references(local_index)
    }

    fn has_link_to(&self, target: WrapperId, other_only: bool) -> bool {
        self.layer.wrapper_users(target).any(|user| {
            (!other_only || user != target)
                && self.binding_row(user).is_some_and(|row| {
                    row.alts
                        .as_slice()
                        .iter()
                        .any(|alt| alt.target == BindingTarget::Link(target))
                })
        })
    }

    fn has_explicit_root_references(&self, local: usize) -> bool {
        if self.detached_holders(local).next().is_some() {
            return true;
        }
        if self.layer.is_empty() {
            return false;
        }
        let root = AllocBase::Root(local as u32);
        self.layer.edges.by_base.contains_key(&root)
            || self.layer.cells_by_base.contains_key(&root)
            || self
                .layer
                .observations_by_base
                .range((root, 0)..=(root, usize::MAX))
                .next()
                .is_some()
            || self.layer.sharing.by_base.contains_key(&root)
            || self
                .layer
                .bound_by_base
                .range(
                    (root, WrapperId::current(0))
                        ..=(root, WrapperId::current(usize::MAX).retired(|_| true)),
                )
                .next()
                .is_some()
            || self
                .layer
                .wrappers_by_base
                .range(
                    (root, WrapperId::current(0))
                        ..=(root, WrapperId::current(usize::MAX).retired(|_| true)),
                )
                .next()
                .is_some()
            || self
                .layer
                .detached_by_base
                .range((root, 0, AllocId::MIN)..=(root, u32::MAX, AllocId::MAX))
                .next()
                .is_some()
    }

    fn can_rebind_uninitialised(&self, local: usize) -> bool {
        self.has_pristine_alloc_binding(local)
            && !self
                .value_root_users
                .range((local, 0)..=(local, usize::MAX))
                .any(|&(_, observer)| {
                    increment_frontend_counter(FrontendCounter::BorrowRetirementLocalRows);
                    observer != local
                        && self
                            .slot_roots(WrapperId::current(observer))
                            .contains(local)
                        && self.binding_mode(observer).contains(LocalMode::SLOT)
                })
            && !self
                .layer
                .slot_root_users
                .range((local, 0)..=(local, usize::MAX))
                .any(|&(_, observer)| observer != local)
    }

    /// A captured entry can retain an old wrapper even without another named link to it.
    pub(in super::super) fn retain_captured_wrapper(&mut self, local: usize) {
        if self.locals[local].init != Init::No {
            self.materialize_prior(WrapperId::current(local));
        }
    }

    fn materialize_prior(&mut self, current: WrapperId) {
        let prior = current.retired(|_| true);
        let mut row = self
            .binding_row(current)
            .cloned()
            .unwrap_or_else(|| BindingRow {
                wrapper: current,
                alts: SmallSet::One(OWN_SLOT),
                coverage: Coverage::Complete,
                route_roots: RootSet::empty(self.locals.len()),
                slot_roots: None,
                routes: Vec::new(),
            });
        row.wrapper = prior;
        if self.locals[current.local].init == Init::Maybe {
            row.coverage = Coverage::Partial;
        }
        if row.has_slot() {
            let current_cell = CellId::Slot(current);
            let prior_cell = CellId::Slot(prior);
            let now = self.layer.contents(current_cell);
            let previous = self.layer.contents(prior_cell);
            if !now.is_empty() || !previous.is_empty() {
                let mut alternatives = if previous.is_empty() {
                    self.implicit_cell_alternatives(prior_cell, now, false)
                } else {
                    previous.to_vec()
                };
                if now.is_empty() {
                    merge_alternatives(
                        &mut alternatives,
                        self.implicit_cell_alternatives(current_cell, previous, false),
                    );
                } else {
                    merge_alternatives(&mut alternatives, now.iter().cloned());
                }
                self.layer.set_contents(prior_cell, alternatives);
            }
            let explicit_prior = !self.layer.bound_of(prior).is_empty();
            let prior_has_views = self.layer.sharing.bindings(prior).next().is_some();
            let mut views: Vec<_> = self.layer.sharing.bindings(current).collect();
            let payload = self.layer.bound_of(current);
            let mut values: Vec<_> = if !payload.is_empty() {
                payload
                    .iter()
                    .map(|&(_, id, readonly)| (prior, id, readonly))
                    .collect()
            } else {
                // The reached Prior wrapper keeps the current value even if nobody has yet
                // looked up its implicit root. Capture before renaming to the retired base.
                self.slot_values(current)
                    .map(|value| (prior, value.id, !value.writable))
                    .collect()
            };
            if prior_has_views && views.is_empty() {
                views.extend(
                    values
                        .iter()
                        .filter_map(|&(_, id, _)| (id != AllocId::EMPTY).then_some(id)),
                );
            }
            if self.owns_slot(prior) {
                let capture_lazy = !values.is_empty() && !explicit_prior;
                let capture_views = !views.is_empty() && !prior_has_views;
                if capture_lazy || capture_views {
                    for old in self.slot_values(prior) {
                        // A bound group hides the old lazy alternative; view correlation
                        // likewise needs the ordinary allocation when only one side has views.
                        if capture_lazy {
                            values.push((prior, old.id, !old.writable));
                        }
                        if capture_views {
                            views.push(old.id);
                        }
                    }
                }
            }
            if !values.is_empty() {
                self.layer.extend_bound(prior, values);
            }
            for view in views {
                self.layer.sharing.edit_row(
                    &self.layer.table.borrow(),
                    SharingRow::Binding(prior, view),
                    false,
                    true,
                );
            }
            row.slot_roots = Some(RootSet::empty(self.locals.len()));
        }
        if let Some(old) = self.binding_row(prior) {
            row.union_row_alts(old);
            row.route_roots.union_with(&old.route_roots);
            if old.coverage == Coverage::Partial {
                row.coverage = Coverage::Partial;
            }
            if row.has_slot() && row.slot_roots.is_none() {
                row.slot_roots.clone_from(&old.slot_roots);
            }
        }
        self.put_binding_row(row);
    }

    fn age_wrappers(
        &mut self,
        locals: impl IntoIterator<Item = usize>,
        retires: impl Fn(usize) -> bool,
    ) {
        let users: BTreeSet<_> = locals
            .into_iter()
            .flat_map(|local| self.layer.wrapper_users(WrapperId::current(local)))
            .collect();
        let layer = &mut self.layer;
        for user in users {
            let Ok(index) = layer
                .wrappers
                .binary_search_by_key(&user, |row| row.wrapper)
            else {
                continue;
            };
            let row = &mut layer.wrappers[index];
            AllocationLayer::edit_wrapper_incidence(
                &mut layer.wrapper_users,
                &mut layer.route_rows,
                row,
                false,
            );
            row.alts.remap(|alt| {
                let target = match alt.target {
                    BindingTarget::Link(source) => BindingTarget::Link(source.retired(&retires)),
                    target => target,
                };
                let origin = alt.origin.map(|origin| Origin {
                    wrapper: origin.wrapper.retired(&retires),
                    owner: origin.owner.map(|owner| owner.retired(&retires)),
                    isolated: origin.isolated,
                });
                BindingAlt {
                    target,
                    origin,
                    ..alt
                }
            });
            AllocationLayer::edit_wrapper_incidence(
                &mut layer.wrapper_users,
                &mut layer.route_rows,
                row,
                true,
            );
        }
    }

    fn freeze_slot_observers(
        &mut self,
        observers: impl Iterator<Item = usize>,
        observes_retired: impl Fn(&RootSet) -> bool,
    ) {
        // Root-only snapshots need an override before the old allocation is renamed.
        // Whole links are not snapshots and still load the cell.
        for observer in observers {
            let slot = WrapperId::current(observer);
            let roots = self.slot_roots(slot);
            if !observes_retired(roots)
                || !self.binding_mode(observer).contains(LocalMode::SLOT)
                || !self.layer.bound_of(slot).is_empty()
            {
                continue;
            }
            let roots = roots.clone();
            let mut table = self.layer.table.borrow_mut();
            let values: Vec<_> = roots
                .iter_ones()
                .map(|root| (slot, table.root(root), false))
                .collect();
            drop(table);
            self.layer.set_bound(slot, values);
        }
    }

    /// Replace the allocation contents without retiring the enclosing storage cell.
    pub(in super::super) fn retire_slot_value(
        &mut self,
        local: usize,
        order: OrderKey,
    ) -> AllocationRetirement {
        let observers = self
            .value_root_users
            .range((local, 0)..=(local, usize::MAX))
            .chain(
                self.layer
                    .slot_root_users
                    .range((local, 0)..=(local, usize::MAX)),
            )
            .filter_map(|&(_, observer)| (observer != local).then_some(observer))
            .collect::<BTreeSet<_>>();
        self.freeze_slot_observers(observers.into_iter(), |roots| roots.contains(local));
        let root = AllocBase::Root(local as u32);
        self.layer.rename_bases(Some(root), |base| {
            (base == AllocBase::Root(local as u32)).then_some(AllocBase::Old(local as u32, order))
        });
        self.invalidate_observations(
            &[CellId::Slot(WrapperId::current(local))],
            Coverage::Complete,
        );
        // The publisher still needs the old occurrence until the replacement is committed.
        // Retirement of the wrapper instead removes it with the Current row.
        self.clear_slot_allocation(WrapperId::current(local));
        self.layer.clear_detachments(local);
        AllocationRetirement {
            local,
            order,
            wrapper: false,
        }
    }

    /// Retain the old occurrence only when another wrapper or an entry capture can reach it.
    pub(in super::super) fn retire_binding_allocation(
        &mut self,
        local: usize,
        order: OrderKey,
    ) -> AllocationRetirement {
        let current = WrapperId::current(local);
        if self.locals[local].init != Init::No && self.has_link_to(current, true) {
            self.materialize_prior(current);
        }
        self.invalidate_observation_routes([current]);
        let retirement = self.retire_slot_value(local, order);
        self.age_wrappers([local], |index| index == local);
        self.remove_current_row(local);
        while let Some(&key) = self
            .derivation_users
            .range((local, 0)..=(local, usize::MAX))
            .next()
        {
            increment_frontend_counter(FrontendCounter::BorrowRetirementLocalRows);
            self.locals[key.1].derived_from_roots.remove(local);
            self.derivation_users.remove(&key);
        }
        AllocationRetirement {
            wrapper: true,
            ..retirement
        }
    }

    fn invalidate_observation_routes(&mut self, retired: impl IntoIterator<Item = WrapperId>) {
        for local in self.linked_observation_locals(retired) {
            self.layer.set_observation(local, None);
        }
    }

    fn linked_observation_locals(
        &self,
        retired: impl IntoIterator<Item = WrapperId>,
    ) -> Vec<usize> {
        let mut locals = Vec::new();
        if self.layer.observations.is_empty() {
            return locals;
        }
        let mut pending: Vec<_> = retired.into_iter().collect();
        let mut seen = FxHashSet::default();
        while let Some(wrapper) = pending.pop() {
            if !seen.insert(wrapper) {
                continue;
            }
            if wrapper.age == WrapperAge::Current
                && self.layer.observations.contains_key(&wrapper.local)
            {
                locals.push(wrapper.local);
            }
            pending.extend(self.layer.wrapper_users(wrapper).filter(|&user| {
                increment_frontend_counter(FrontendCounter::BorrowRetirementWrapperRows);
                self.binding_row(user).is_some_and(|row| {
                    row.alts
                        .as_slice()
                        .iter()
                        .any(|alt| alt.target == BindingTarget::Link(wrapper))
                })
            }));
        }
        locals
    }

    /// Apply a simultaneous set of edge retirements to a pre-edge allocation capture.
    pub(in super::super) fn remap_retired_alloc_id(
        &self,
        id: AllocId,
        order: OrderKey,
        retires: impl FnOnce(usize) -> bool,
    ) -> AllocId {
        if id == AllocId::EMPTY {
            return id;
        }
        let mut table = self.layer.table.borrow_mut();
        match table.base(id) {
            AllocBase::Root(local) if retires(local as usize) => {
                table.rebase(id, AllocBase::Old(local, order), false)
            }
            _ => id,
        }
    }

    /// Store `values` into `key` of every holder.
    ///
    /// WHAT: a strong store replaces the group; a weak store adds to it, keeping the lazy child
    ///       when the group had no explicit edge.
    pub(in super::super) fn store_alloc_field(
        &mut self,
        holders: &[AllocId],
        key: FieldKey,
        values: &[StoredValue],
        strong: bool,
    ) {
        self.store_alloc_field_with_views(holders, key, values, &[], strong, true);
    }

    pub(in super::super) fn store_alloc_field_with_views(
        &mut self,
        holders: &[AllocId],
        key: FieldKey,
        values: &[StoredValue],
        views: &[AllocId],
        strong: bool,
        distinct_occurrence: bool,
    ) {
        if holders.is_empty() || (!strong && values.is_empty()) {
            return;
        }
        self.store_view_projections(holders, key, values, views, strong);
        if !values.is_empty() {
            self.invalidate_detachments();
        }
        for &holder in holders {
            self.prepare_edge_group(holder, key, strong, true);
            for value in values {
                let single = distinct_occurrence && self.alloc_singular(value.id);
                self.layer.insert_edge(
                    Edge {
                        holder,
                        key,
                        held: value.id,
                        writable: value.writable,
                        single,
                    },
                    distinct_occurrence,
                );
            }
            if strong && values.is_empty() {
                self.layer.add_edge(Edge {
                    holder,
                    key,
                    held: AllocId::EMPTY,
                    writable: true,
                    single: true,
                });
            }
        }
    }

    fn prepare_edge_group(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        strong: bool,
        supersede_empty: bool,
    ) {
        if strong {
            self.layer
                .edges
                .remove_group(&self.layer.table, holder, key, false);
        } else if self.layer.edges.group(holder, key).is_empty() {
            let lazy = self.layer.table.borrow_mut().child(holder, key);
            self.layer.add_edge(Edge {
                holder,
                key,
                held: lazy,
                writable: true,
                single: false,
            });
        } else if supersede_empty {
            // New members supersede the empty marker, not other possible old members.
            self.layer
                .edges
                .remove_group(&self.layer.table, holder, key, true);
        }
    }

    #[allow(dead_code)] // Prepared publisher for typed final-member postconditions.
    pub(in super::super) fn publish_family_members(
        &mut self,
        receivers: &[AllocId],
        coverage: Coverage,
        key: FieldKey,
        replace: Replacement,
        members: &[FamilyMember],
    ) -> PublicationCarriers {
        debug_assert!(matches!(key, FieldKey::Elem | FieldKey::MapKey));
        let mut receipt = PublicationCarriers::default();
        let definite_receiver = coverage == Coverage::Complete
            && matches!(receivers, [receiver] if self.alloc_singular(*receiver));
        let exact = replace == Replacement::Must && definite_receiver;
        if members.is_empty() && replace == Replacement::May {
            return receipt;
        }
        let cells: Vec<_> = receivers
            .iter()
            .map(|&receiver| CellId::Projected(receiver, key))
            .collect();
        self.invalidate_observations(&cells, coverage);
        if members.is_empty() && !exact {
            // A non-definite replacement can clear any candidate. Keep possible
            // old members and add the empty outcome alongside them.
            for cell in cells {
                let CellId::Projected(holder, key) = cell else {
                    unreachable!()
                };
                if !self.layer.contents(cell).is_empty() {
                    self.publish_cell_contents(
                        cell,
                        &[],
                        &[ValueAlt::plain(ValueKind::Absent)],
                        false,
                        false,
                    );
                }
                self.prepare_edge_group(holder, key, false, false);
                self.layer.add_edge(Edge {
                    holder,
                    key,
                    held: AllocId::EMPTY,
                    writable: true,
                    single: false,
                });
            }
            return receipt;
        }
        let mut values = Vec::new();
        let mut views = Vec::new();
        let mut alternatives = Vec::new();
        let has_views = members.iter().any(|member| !member.value.views.is_empty());
        let explicit = members
            .iter()
            .any(|member| !member.value.alternatives.is_empty() || member.value.values.is_empty());
        for member in members {
            values.extend_from_slice(&member.value.values);
            if has_views && member.value.views.is_empty() {
                // An explicit view group masks the ordinary edge fallback, so it must
                // also contain a representative for each plain member.
                views.extend(member.value.values.iter().map(|value| value.id));
            } else {
                views.extend_from_slice(&member.value.views);
            }
            if explicit && member.value.alternatives.is_empty() {
                if member.value.values.is_empty() {
                    alternatives.push(ValueAlt::plain(ValueKind::Scalar));
                } else {
                    alternatives.extend(
                        member
                            .value
                            .values
                            .iter()
                            .copied()
                            .map(|value| ValueAlt::plain(ValueKind::Allocation(value))),
                    );
                }
            } else {
                alternatives.extend(member.value.alternatives.iter().cloned());
            }
        }
        let possibly_absent = alternatives.iter().any(|alt| alt.kind == ValueKind::Absent);
        let must_add_member = definite_receiver && !members.is_empty() && !possibly_absent;
        for cell in cells {
            receipt.record(cell, self.layer.contents(cell), cell, &alternatives, exact);
            self.publish_cell_contents(cell, &values, &alternatives, exact, false);
            if must_add_member
                && self
                    .layer
                    .contents(cell)
                    .iter()
                    .any(|alt| alt.kind == ValueKind::Absent)
                && let Some((cell, mut retained)) = self.layer.take_contents(cell)
            {
                retained.retain(|alt| alt.kind != ValueKind::Absent);
                self.layer.set_contents(cell, retained);
            }
        }
        self.store_view_projections(receivers, key, &values, &views, exact);
        if !values.is_empty() {
            self.invalidate_detachments();
        }
        for &holder in receivers {
            self.prepare_edge_group(holder, key, exact, must_add_member);
            for member in members {
                for value in &member.value.values {
                    self.layer.insert_edge(
                        Edge {
                            holder,
                            key,
                            held: value.id,
                            writable: value.writable,
                            single: definite_receiver
                                && member.at_most_once
                                && self.alloc_singular(value.id),
                        },
                        true,
                    );
                }
            }
            if (exact && values.is_empty()) || possibly_absent {
                self.layer.add_edge(Edge {
                    holder,
                    key,
                    held: AllocId::EMPTY,
                    writable: true,
                    single: true,
                });
            }
        }
        receipt
    }

    fn store_view_projections(
        &mut self,
        holders: &[AllocId],
        key: FieldKey,
        values: &[StoredValue],
        views: &[AllocId],
        strong: bool,
    ) {
        if self.layer.sharing.is_empty() && views.is_empty() {
            return;
        }
        let sources = &self.layer.sharing;
        let mut affected: Vec<_> = holders
            .iter()
            .flat_map(|&holder| {
                sources.views_for(holder).inspect(|_| {
                    increment_frontend_counter(FrontendCounter::BorrowPublisherGroupRowsInspected);
                })
            })
            .collect();
        affected.extend_from_slice(holders);
        affected.sort_unstable();
        affected.dedup();
        for view in affected {
            let existing = self.layer.sharing.projections(view, key).next().is_some();
            if !existing && views.is_empty() {
                continue;
            }
            let mut exact = strong;
            self.layer
                .with_members(view, |id| exact &= holders.contains(&id));
            if exact {
                self.layer
                    .sharing
                    .clear_projection(&self.layer.table.borrow(), view, key);
            } else if !existing {
                let mut previous = Vec::new();
                self.layer.project_members(
                    &mut self.layer.table.borrow_mut(),
                    view,
                    key,
                    &mut previous,
                );
                for id in previous {
                    self.link_view(view, key, id);
                }
            }
            if views.is_empty() {
                for value in values {
                    self.link_view(view, key, value.id);
                }
            } else {
                for &held in views {
                    self.link_view(view, key, held);
                }
            }
        }
    }

    /// Record one construction child occurrence.
    #[cfg(test)]
    pub(in super::super) fn add_alloc_edge(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        value: StoredValue,
    ) {
        self.record_constructed_field(holder, key, std::slice::from_ref(&value), &[], true);
    }

    /// Publish a summarized relationship, not a count of concrete insertions.
    pub(in super::super) fn contribute_alloc_edge(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        value: StoredValue,
    ) {
        self.record_constructed_field(holder, key, std::slice::from_ref(&value), &[], false);
    }

    /// Every concrete child of a tracked field also needs an occurrence. Keep ordinary
    /// children before or after the first tracked child under the same collapsed selector.
    pub(in super::super) fn record_constructed_field(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        values: &[StoredValue],
        views: &[AllocId],
        distinct_occurrence: bool,
    ) {
        self.record_constructed_contents(holder, key, values, views, &[], distinct_occurrence);
    }

    fn record_constructed_contents(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        values: &[StoredValue],
        views: &[AllocId],
        alternatives: &[ValueAlt],
        distinct_occurrence: bool,
    ) {
        self.publish_cell_contents(
            CellId::Projected(holder, key),
            values,
            alternatives,
            false,
            true,
        );
        let tracked = self.layer.sharing.projections(holder, key).next().is_some();
        if !tracked && !views.is_empty() {
            let previous: Vec<_> = self
                .layer
                .edges
                .group(holder, key)
                .iter()
                .filter_map(|edge| (edge.held != AllocId::EMPTY).then_some(edge.held))
                .collect();
            for id in previous {
                self.link_view(holder, key, id);
            }
        }
        if !views.is_empty() {
            for &view in views {
                self.link_view(holder, key, view);
            }
        } else if tracked {
            for value in values {
                self.link_view(holder, key, value.id);
            }
        }
        for &value in values {
            self.insert_alloc_edge(holder, key, value, distinct_occurrence);
        }
    }

    pub(in super::super) fn record_constructed_value(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        value: &CapturedValue,
        distinct_occurrence: bool,
    ) {
        self.record_constructed_contents(
            holder,
            key,
            &value.values,
            &value.views,
            &value.alternatives,
            distinct_occurrence,
        );
    }

    fn insert_alloc_edge(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        value: StoredValue,
        distinct_occurrence: bool,
    ) {
        let table = self.layer.table.borrow();
        let fresh = table.singular(holder);
        let single = table.singular(value.id);
        drop(table);
        // A fresh graph cannot add reachability to either side of an established
        // detachment. A construction site in a loop may already have live instances.
        if !fresh {
            self.invalidate_detachments();
        }
        self.layer.insert_edge(
            Edge {
                holder,
                key,
                held: value.id,
                writable: value.writable,
                single: single && distinct_occurrence,
            },
            distinct_occurrence,
        );
    }

    pub(in super::super) fn alloc_child(&self, parent: AllocId, key: FieldKey) -> AllocId {
        self.layer.table.borrow_mut().child(parent, key)
    }

    pub(in super::super) fn has_alloc_edges(&self) -> bool {
        !self.layer.edges.is_empty()
    }

    /// Copy every recorded relationship into fresh writable storage.
    ///
    /// Acyclic evaluations name each possible source graph separately. A loop site uses
    /// selector paths instead of source-derived identities: copying an earlier iteration's
    /// value must not generate another identity for the same allocation site.
    pub(in super::super) fn copy_alloc_graph(
        &mut self,
        source: &ResolvedPlace,
        site: AllocId,
    ) -> Vec<AllocId> {
        let sources = source.ids.as_slice();
        if sources.is_empty() || self.layer.edges.is_empty() {
            return Vec::new();
        }
        let correlated = !source.views.is_empty()
            || (!self.layer.sharing.projections.is_empty()
                && self.alloc_reaches_where(sources, false, |table, current| {
                    self.layer
                        .sharing
                        .projections
                        .keys()
                        .any(|&(holder, _)| self.layer.covers(table, current, holder))
                }));
        let mut table = self.layer.table.borrow_mut();
        let singular = table.singular(site);
        let mut copied = Vec::new();
        let mut roots = Vec::new();
        if correlated {
            let views = if source.views.is_empty() {
                sources
            } else {
                &source.views
            };
            for &view in views {
                let source_key = if singular { view } else { site };
                let base = table.base(site);
                let root =
                    table.intern(AllocKey::CopiedOccurrence(site, source_key), base, singular);
                self.layer
                    .copy_correlated_edges(&mut table, site, root, view, &mut copied);
                roots.push(root);
            }
            roots.dedup();
        } else if singular {
            for &source in sources {
                let root = table.copied_node(site, source, true);
                self.layer
                    .copy_edges(&mut table, [(source, root)], root, &mut copied);
                roots.push(root);
            }
        } else {
            self.layer.copy_edges(
                &mut table,
                sources.iter().map(|&source| (source, site)),
                site,
                &mut copied,
            );
            roots.push(site);
        }
        drop(table);
        if copied.is_empty() {
            return Vec::new();
        }
        if !singular {
            // A previous iteration's graph may still be reachable. Its summary gained edges.
            self.invalidate_detachments();
        }
        copied.sort_unstable_by_key(Edge::sort_key);
        for edge in copied {
            self.layer.insert_edge(edge, false);
        }
        roots
    }

    pub(in super::super) fn alloc_group_is_explicit(&self, holder: AllocId, key: FieldKey) -> bool {
        !self.layer.edges.group(holder, key).is_empty()
            || !self
                .layer
                .contents(CellId::Projected(holder, key))
                .is_empty()
    }

    pub(in super::super) fn value_cells(&self) -> impl Iterator<Item = (CellId, &[ValueAlt])> + '_ {
        self.layer
            .contents
            .iter()
            .map(|(cell, alternatives)| (*cell, alternatives.as_slice()))
    }

    pub(in super::super) fn value_cell(&self, cell: CellId) -> &[ValueAlt] {
        self.layer.contents(cell)
    }

    /// Preserve explicit absence even when the same slot may hold an allocation,
    /// without resolving an implicit root in the interner.
    #[allow(dead_code)]
    pub(in super::super) fn retained_slot_payload(
        &self,
        slot: WrapperId,
    ) -> Option<impl Iterator<Item = ValueKind> + '_> {
        let payload = self.layer.bound_of(slot);
        (!payload.is_empty()).then(|| {
            payload
                .iter()
                .map(|&(_, id, readonly)| stored_value_kind(id, !readonly))
        })
    }

    /// Raw explicit edge groups preserve the distinction between an absent
    /// group and the empty sentinel. They never intern a lazy child.
    #[allow(dead_code)] // The boundary quotient walks just the reached holders.
    pub(in super::super) fn retained_edge_groups(
        &self,
        holder: AllocId,
    ) -> impl Iterator<Item = &[Edge]> {
        self.layer.edges.holder_groups(holder)
    }

    /// The exceptional rows include dependency-carrying scalars without an
    /// allocation edge. Export needs both projections of the same cell.
    #[allow(dead_code)]
    pub(in super::super) fn retained_projected_values(
        &self,
        holder: AllocId,
    ) -> impl Iterator<Item = (FieldKey, &[ValueAlt])> {
        self.layer
            .contents
            .range(
                CellId::Projected(holder, FieldKey::Field(0))
                    ..=CellId::Projected(holder, FieldKey::MapKey),
            )
            .filter_map(|(cell, values)| match cell {
                CellId::Projected(_, key) => Some((*key, values.as_slice())),
                CellId::Slot(_) => None,
            })
    }

    #[allow(dead_code)]
    pub(in super::super) fn retained_projection_views(
        &self,
        view: AllocId,
        selector: FieldKey,
    ) -> impl Iterator<Item = AllocId> + '_ {
        self.layer.sharing.projections(view, selector)
    }

    pub(in super::super) fn retained_value_routes(
        &self,
        owner: WrapperId,
    ) -> &[(CellId, DependencySet)] {
        self.binding_row(owner)
            .map_or(&[], |row| row.routes.as_slice())
    }
    pub(in super::super) fn value_routes(
        &self,
    ) -> impl Iterator<Item = (WrapperId, CellId, &DependencySet)> + '_ {
        (self.layer.route_rows != 0)
            .then_some(self.layer.wrappers.as_slice())
            .into_iter()
            .flatten()
            .flat_map(|row| {
                row.routes.iter().filter_map(|(cell, deps)| {
                    (!deps.is_empty()).then_some((row.wrapper, *cell, deps))
                })
            })
    }

    pub(in super::super) fn value_route_owners(&self, local: usize) -> FxHashSet<WrapperId> {
        if self.layer.route_rows != 0 && self.has_wrapped_binding(local) {
            self.reachable_from(vec![WrapperId::current(local)])
        } else {
            FxHashSet::default()
        }
    }

    /// A must observation of an ambiguous destination can exclude otherwise possible values.
    pub(in super::super) fn observed_payload(&self, local: usize) -> Option<&[ValueAlt]> {
        self.layer
            .observation(local)
            .map(|observation| observation.alternatives.as_slice())
    }

    pub(in super::super) fn visible_allocation(&self, id: AllocId, visible: &RootSet) -> AllocId {
        if id == AllocId::EMPTY {
            return id;
        }
        let mut table = self.layer.table.borrow_mut();
        if let Some(base) = departed_base(table.base(id), visible) {
            table.rebase(id, base, false)
        } else {
            id
        }
    }

    /// Publish a captured payload without re-evaluating the operation's destination.
    pub(in super::super) fn publish_store(
        &mut self,
        operation: &TypedStore,
        order: OrderKey,
        retires: Option<&dyn Fn(usize) -> bool>,
    ) -> StoreReceipt {
        let remap = |state: &Self, id| {
            retires.map_or(id, |retire| state.remap_retired_alloc_id(id, order, retire))
        };
        match operation {
            TypedStore::EraseFamily {
                receivers,
                coverage,
                keys,
            } => {
                if let [receiver] = receivers.as_slice()
                    && *coverage == Coverage::Complete
                {
                    let receiver = remap(self, *receiver);
                    if self.alloc_singular(receiver) {
                        return self.erase_alloc_family(receiver, *keys);
                    }
                }
                // A conditional erase keeps old members in the may graph, but does not
                // preserve a must observation of a member that may have been removed.
                if !self.layer.observations.is_empty() {
                    let cells: Vec<_> = receivers
                        .iter()
                        .flat_map(|&receiver| {
                            let holder = remap(self, receiver);
                            [
                                CellId::Projected(holder, FieldKey::Elem),
                                CellId::Projected(holder, FieldKey::MapKey),
                            ]
                        })
                        .collect();
                    self.invalidate_observations(&cells, *coverage);
                }
                StoreReceipt::default()
            }
            TypedStore::Write(write) => self.publish_write(write, order, retires, None),
        }
    }

    #[allow(dead_code)]
    pub(in super::super) fn publish_captured_store(
        &mut self,
        operation: &TypedStore,
        order: OrderKey,
    ) -> PublicationCarriers {
        let mut carriers = PublicationCarriers::default();
        let TypedStore::Write(write) = operation else {
            unreachable!("whole-family effects are published with their final members");
        };
        self.publish_write(write, order, None, Some(&mut carriers));
        carriers
    }

    fn publish_write(
        &mut self,
        write: &TypedWrite,
        order: OrderKey,
        retires: Option<&dyn Fn(usize) -> bool>,
        mut carriers: Option<&mut PublicationCarriers>,
    ) -> StoreReceipt {
        let TypedWrite {
            addresses,
            value,
            replace,
            distinct_occurrence,
            observer,
            slot_roots,
            batch_receiver,
            explicit_slot_payload,
        } = write;
        let remap =
            |id| retires.map_or(id, |retire| self.remap_retired_alloc_id(id, order, retire));
        let remap_cell = |cell: CellId| {
            cell.remap(
                |wrapper| wrapper.retired(|local| retires.is_some_and(|retire| retire(local))),
                remap,
            )
        };
        // Partial coverage withholds a kill, not permission to write outside the capture.
        let mut cells = remap_invoke_capture(addresses.cells(), remap_cell);
        if let Cow::Owned(cells) = &mut cells {
            cells.sort_unstable();
            cells.dedup();
        }
        let definite = if let [cell] = cells.as_ref()
            && *replace == Replacement::Must
            && addresses.definite().is_some()
        {
            match *cell {
                CellId::Slot(wrapper) if wrapper.age == WrapperAge::Current => Some(*cell),
                CellId::Projected(holder, key) if self.projected_cell_is_exact(holder, key) => {
                    Some(*cell)
                }
                _ => None,
            }
        } else {
            None
        };
        let values = remap_invoke_capture(&value.values, |mut value| {
            value.id = remap(value.id);
            value
        });
        let views = remap_invoke_capture(&value.views, remap);
        let mut alternatives = Cow::Borrowed(value.alternatives.as_slice());
        if !alternatives.is_empty() && retires.is_some() {
            let mut rebased = value.alternatives.clone();
            remap_value_alternatives(&mut rebased, remap);
            alternatives = Cow::Owned(rebased);
        }
        let replaced = if !alternatives.is_empty() || !self.layer.contents.is_empty() {
            let replaced = definite.and_then(|cell| self.layer.take_contents(cell));
            for &cell in cells.as_ref() {
                if let Some(carriers) = carriers.as_deref_mut() {
                    let old = if definite == Some(cell) {
                        replaced
                            .as_ref()
                            .map_or(&[][..], |(_, values)| values.as_slice())
                    } else {
                        self.layer.contents(cell)
                    };
                    carriers.record(cell, old, cell, &alternatives, definite == Some(cell));
                }
                self.publish_cell_contents(
                    cell,
                    &values,
                    &alternatives,
                    definite == Some(cell),
                    false,
                );
            }
            replaced
        } else {
            None
        };
        self.invalidate_observations(&cells, addresses.coverage);

        let readonly = observer.is_some_and(|(_, readonly)| readonly);
        if *batch_receiver {
            match cells.as_ref() {
                [CellId::Projected(holder, key)] => self.store_alloc_field_with_views(
                    std::slice::from_ref(holder),
                    *key,
                    &values,
                    &views,
                    false,
                    *distinct_occurrence,
                ),
                [CellId::Projected(_, key), ..] => {
                    let holders: Vec<_> = cells
                        .iter()
                        .filter_map(|&cell| match cell {
                            CellId::Projected(holder, _) => Some(holder),
                            CellId::Slot(_) => None,
                        })
                        .collect();
                    self.store_alloc_field_with_views(
                        &holders,
                        *key,
                        &values,
                        &views,
                        false,
                        *distinct_occurrence,
                    );
                }
                _ => {}
            }
        } else {
            for &cell in cells.as_ref() {
                let exact = definite == Some(cell);
                match cell {
                    CellId::Slot(slot) => self.store_slot_payload(
                        slot,
                        &values,
                        &views,
                        slot_roots.as_ref(),
                        exact,
                        readonly,
                        *explicit_slot_payload,
                    ),
                    CellId::Projected(holder, key) => self.store_alloc_field_with_views(
                        &[holder],
                        key,
                        &values,
                        &views,
                        exact,
                        *distinct_occurrence,
                    ),
                }
            }
        }
        if let Some((local, readonly)) = *observer
            && addresses.coverage == Coverage::Complete
            && !cells.is_empty()
            && self.binding_mode(local).contains(LocalMode::ALIAS)
            && self.resolve_cells(local).cells() == cells.as_ref()
        {
            if let Some(roots) = slot_roots {
                let roots = if roots.is_empty() {
                    // Report fresh payload origins rather than possible old value roots.
                    self.binding(local).attribution_roots()
                } else {
                    roots.clone()
                };
                self.set_local_value_roots(local, roots);
            }
            if definite.is_none() {
                let mut entry = Observation {
                    local,
                    values: values.into_owned(),
                    views: views.into_owned(),
                    alternatives: alternatives.into_owned(),
                    readonly,
                };
                normalize_observation(&mut entry);
                self.record_observation(entry);
            }
        }
        StoreReceipt {
            replaced: [replaced, None],
        }
    }

    fn publish_cell_contents(
        &mut self,
        cell: CellId,
        values: &[StoredValue],
        incoming: &[ValueAlt],
        exact: bool,
        constructing: bool,
    ) {
        let old = self.layer.contents(cell);
        if incoming.is_empty() && (old.is_empty() || exact) {
            if !old.is_empty() {
                self.layer.set_contents(cell, Vec::new());
            }
            return;
        }
        let mut alternatives = if exact {
            Vec::new()
        } else if !old.is_empty() {
            old.to_vec()
        } else {
            self.implicit_cell_alternatives(cell, incoming, constructing)
        };
        if incoming.is_empty() {
            merge_alternatives(
                &mut alternatives,
                compact_alternatives(values.iter().copied()),
            );
        } else {
            merge_alternatives(&mut alternatives, incoming.iter().cloned());
        }
        self.layer.set_contents(cell, alternatives);
    }

    fn implicit_cell_alternatives(
        &self,
        cell: CellId,
        incoming: &[ValueAlt],
        constructing: bool,
    ) -> ValueAlternatives {
        let scalar = incoming.iter().any(|alt| alt.kind == ValueKind::Scalar);
        match cell {
            CellId::Slot(slot) if !self.owns_slot(slot) => Vec::new(),
            CellId::Slot(_) if scalar => vec![ValueAlt::plain(ValueKind::Scalar)],
            CellId::Slot(slot) => compact_alternatives(self.slot_values(slot)),
            CellId::Projected(holder, key) => {
                let edges = self.layer.edges.group(holder, key);
                if edges.is_empty() {
                    if constructing {
                        Vec::new()
                    } else if scalar {
                        vec![ValueAlt::plain(ValueKind::Scalar)]
                    } else {
                        vec![ValueAlt::plain(ValueKind::Allocation(StoredValue {
                            id: self.alloc_child(holder, key),
                            writable: true,
                        }))]
                    }
                } else {
                    edges.iter().map(Edge::value_alternative).collect()
                }
            }
        }
    }

    fn record_observation(&mut self, observation: Observation) {
        let local = observation.local;
        let routes = observation_routes(&self.resolve_cells(local));
        self.layer.set_observation(local, Some(observation));
        self.layer.set_observation_routes(local, routes);
    }

    pub(super) fn refresh_linked_observations(&mut self, wrapper: WrapperId) {
        if self.layer.observations.is_empty()
            || !(wrapper.age == WrapperAge::Current
                && self.layer.observations.contains_key(&wrapper.local)
                || self.layer.wrapper_users(wrapper).next().is_some())
        {
            return;
        }
        for local in self.linked_observation_locals([wrapper]) {
            let address = self.resolve_cells(local);
            self.layer
                .set_observation_routes(local, observation_routes(&address));
        }
    }

    fn rebuild_observation_routes(&mut self) {
        self.layer.observation_routes.clear();
        self.layer.observations_by_route.clear();
        let entries: Vec<_> = self
            .layer
            .observations
            .keys()
            .map(|&local| (local, observation_routes(&self.resolve_cells(local))))
            .collect();
        for (local, routes) in entries {
            self.layer.set_observation_routes(local, routes);
        }
    }

    fn invalidate_observations(&mut self, cells: &[CellId], coverage: Coverage) {
        if self.layer.observations.is_empty() {
            return;
        }
        if coverage == Coverage::Partial {
            self.layer.clear_observations();
            return;
        }
        let candidates = self.observation_candidates(cells);
        let removed: Vec<_> = candidates
            .into_iter()
            .filter(|&local| {
                increment_frontend_counter(FrontendCounter::BorrowRetirementCellRows);
                let observed = self.resolve_cells(local);
                observed.coverage != Coverage::Complete
                    || observed.cells().iter().any(|&left| {
                        cells.iter().any(|&right| match (left, right) {
                            (CellId::Slot(a), CellId::Slot(b)) => a == b,
                            (CellId::Projected(a, x), CellId::Projected(b, y)) => {
                                x == y && self.alloc_paths_may_share(&[a], &[], &[b], &[])
                            }
                            _ => false,
                        })
                    })
            })
            .collect();
        for local in removed {
            self.layer.set_observation(local, None);
        }
    }

    fn observation_candidates(&self, cells: &[CellId]) -> BTreeSet<usize> {
        cells
            .iter()
            .map(|cell| match *cell {
                CellId::Slot(slot) => ObservationRoute::Slot(slot),
                CellId::Projected(_, key) => ObservationRoute::Selector(key),
            })
            .chain(std::iter::once(ObservationRoute::Incomplete))
            .flat_map(|route| {
                self.layer
                    .observations_by_route
                    .range((route, 0)..=(route, usize::MAX))
                    .map(|&(_, local)| local)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn store_slot_payload(
        &mut self,
        slot: WrapperId,
        values: &[StoredValue],
        views: &[AllocId],
        roots: Option<&RootSet>,
        exact: bool,
        readonly: bool,
        explicit_slot_payload: bool,
    ) {
        if !exact && values.is_empty() && roots.is_none_or(RootSet::is_empty) {
            return;
        }
        let mut retained: Vec<_> = if exact {
            Vec::new()
        } else {
            self.layer.sharing.bindings(slot).collect()
        };
        let old = if !exact
            && !values.is_empty()
            && (self.layer.bound_of(slot).is_empty() || retained.is_empty() && !views.is_empty())
        {
            self.slot_values(slot).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if slot.age == WrapperAge::Current {
            if let Some(roots) = roots
                && (exact || !roots.is_empty())
            {
                let merged = if exact {
                    roots.clone()
                } else {
                    let mut merged = self.slot_roots(slot).clone();
                    if merged.is_empty() {
                        merged.insert(slot.local);
                    }
                    merged.union_with(roots);
                    merged
                };
                if let Ok(index) = self
                    .layer
                    .wrappers
                    .binary_search_by_key(&slot, |row| row.wrapper)
                {
                    let row = &mut self.layer.wrappers[index];
                    AllocationLayer::index_wrapper_slot_roots(
                        &mut self.layer.slot_root_users,
                        row,
                        false,
                    );
                    row.slot_roots = Some(merged.clone());
                    AllocationLayer::index_wrapper_slot_roots(
                        &mut self.layer.slot_root_users,
                        row,
                        true,
                    );
                    let mut roots = merged;
                    roots.union_with(&self.locals[slot.local].value_roots);
                    self.set_local_value_roots(slot.local, roots);
                } else {
                    self.set_local_value_roots(slot.local, merged);
                }
            }
            self.layer.clear_detachments(slot.local);
        }
        if exact && values.is_empty() && views.is_empty() && !readonly && !explicit_slot_payload {
            // Plain scalar updates keep the implicit slot and allocate no sparse facts.
            self.clear_slot_allocation(slot);
            return;
        }
        if !exact && retained.is_empty() && !views.is_empty() {
            retained.extend(old.iter().map(|value| value.id));
        }
        if exact || !values.is_empty() {
            let bound = self.layer.bound_of(slot);
            let incoming = |value: &StoredValue| (slot, value.id, readonly || !value.writable);
            let mut group = match (exact, values) {
                (true, []) => vec![(slot, AllocId::EMPTY, readonly)],
                (true, [value]) => vec![incoming(value)],
                _ => {
                    // Retiring captures can reorder or collapse ids, so normalise this slot only.
                    let mut group = Vec::with_capacity(
                        values.len() + if exact { 0 } else { bound.len().max(old.len()) },
                    );
                    if !exact {
                        if bound.is_empty() {
                            group.extend(old.iter().map(|value| (slot, value.id, !value.writable)));
                        } else {
                            group.extend_from_slice(bound);
                        }
                    }
                    group.extend(values.iter().map(incoming));
                    group
                }
            };
            normalize_bindings(&mut group);
            self.layer.set_bound(slot, group);
        }
        if retained.is_empty() {
            if exact || !views.is_empty() {
                self.bind_slot_views(slot, views);
            }
        } else {
            if views.is_empty() {
                retained.extend(values.iter().map(|value| value.id));
            } else {
                retained.extend_from_slice(views);
            }
            self.bind_slot_views(slot, &retained);
        }
    }

    fn erase_alloc_family(&mut self, holder: AllocId, keys: EntryFamilies) -> StoreReceipt {
        self.invalidate_observations(
            &[
                CellId::Projected(holder, FieldKey::Elem),
                CellId::Projected(holder, FieldKey::MapKey),
            ],
            Coverage::Complete,
        );
        self.store_alloc_field(&[holder], FieldKey::Elem, &[], true);
        let element = self
            .layer
            .take_contents(CellId::Projected(holder, FieldKey::Elem));
        let key = if matches!(keys, EntryFamilies::MapEntries) {
            self.store_alloc_field(&[holder], FieldKey::MapKey, &[], true);
            self.layer
                .take_contents(CellId::Projected(holder, FieldKey::MapKey))
        } else {
            None
        };
        StoreReceipt {
            replaced: [element, key],
        }
    }

    /// Remove an element or map entry after a successful removal.
    ///
    /// WHAT: a single holder whose group holds exactly one allocation, once, is left empty.
    /// WHY: that group had one element, so the removal took it. Any other shape may leave a
    ///      copy behind, so the group stays as it was.
    pub(in super::super) fn remove_alloc_element(
        &mut self,
        holders: &[AllocId],
        map: bool,
        result: Option<usize>,
    ) -> StoreReceipt {
        let [holder] = holders else {
            return StoreReceipt::default();
        };
        let group = self.layer.edges.group(*holder, FieldKey::Elem);
        if let [edge] = group
            && edge.single
            && edge.held != AllocId::EMPTY
            && self.alloc_singular(*holder)
        {
            let holder = *holder;
            let receipt = self.erase_alloc_family(
                holder,
                if map {
                    EntryFamilies::MapEntries
                } else {
                    EntryFamilies::Elements
                },
            );
            if let Some(result) = result
                && !self.graphs_overlap(&[holder], &self.local_alloc_ids(result).0)
            {
                self.bind_detached_holders(result, [holder]);
            }
            return receipt;
        }
        StoreReceipt::default()
    }

    /// Test typed reach in the given direction, without a coarse provenance precondition.
    pub(in super::super) fn alloc_reaches_typed(
        &self,
        ids: &[AllocId],
        target: AllocId,
        deep: bool,
        writable_only: bool,
    ) -> bool {
        if !deep {
            return ids.contains(&target);
        }
        self.alloc_reaches_where(ids, writable_only, |table, current| {
            self.layer
                .covers_through(table, current, target, writable_only)
        })
    }

    fn alloc_reaches_where(
        &self,
        from: &[AllocId],
        writable_only: bool,
        matches: impl Fn(&AllocTable, AllocId) -> bool,
    ) -> bool {
        self.walk_explicit_allocations(from, writable_only, writable_only, |table, id| {
            if matches(table, id) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    /// An opaque result can observe a reachable child without a contract row naming that child.
    pub(in super::super) fn extend_possible_dependencies(
        &self,
        value: &CapturedValue,
        dependencies: &mut DependencySet,
    ) {
        dependencies.extend(
            value
                .alternatives
                .iter()
                .flat_map(|alt| alt.deps.iter().cloned()),
        );
        if value.values.is_empty() || self.layer.contents.is_empty() {
            return;
        }
        let roots: Vec<_> = value.values.iter().map(|stored| stored.id).collect();
        let children = self.alloc_readable_closure(&roots);
        let table = self.layer.table.borrow();
        for (cell, alternatives) in &self.layer.contents {
            if let CellId::Projected(holder, _) = *cell
                && roots
                    .iter()
                    .chain(&children)
                    .any(|&root| self.layer.covers(&table, root, holder))
            {
                dependencies.extend(alternatives.iter().flat_map(|alt| alt.deps.iter().cloned()));
            }
        }
    }

    /// Allocations reachable from `ids` through writable explicit edges, excluding `ids`.
    pub(in super::super) fn alloc_writable_closure(&self, ids: &[AllocId]) -> Vec<AllocId> {
        self.alloc_closure(ids, true)
    }

    pub(in super::super) fn alloc_readable_closure(&self, ids: &[AllocId]) -> Vec<AllocId> {
        self.alloc_closure(ids, false)
    }

    fn alloc_closure(&self, ids: &[AllocId], writable_only: bool) -> Vec<AllocId> {
        let mut seeds = ids.len();
        let mut closure = Vec::new();
        let _ = self.walk_explicit_allocations(ids, writable_only, false, |_, id| {
            if seeds == 0 {
                closure.push(id);
            } else {
                seeds -= 1;
            }
            ControlFlow::Continue(())
        });
        closure
    }

    /// Join sparse facts from the pre-join inputs, then publish the merged local facts.
    pub(in super::super) fn join_in_place(&mut self, other: &Self) -> bool {
        let changed = self.join_layer(other);
        let wrappers = self.merged_wrappers(other);
        let locals_changed = self.join_local_facts(other);
        let routing_changed = locals_changed || wrappers.is_some();
        if let Some(wrappers) = wrappers {
            self.layer.replace_wrappers(wrappers);
        }
        if routing_changed && !self.layer.observations.is_empty() {
            self.rebuild_observation_routes();
        }
        changed || routing_changed
    }

    fn join_layer(&mut self, other: &Self) -> bool {
        if self.layer.is_empty() && other.layer.is_empty() {
            return false;
        }
        let mut changed = self.join_sharing(other);
        if self.layer.contents != other.layer.contents {
            let mut cells: Vec<_> = self
                .layer
                .contents
                .iter()
                .chain(&other.layer.contents)
                .map(|(&cell, _)| cell)
                .collect();
            cells.sort_unstable();
            cells.dedup();
            let contents = cells
                .into_iter()
                .map(|cell| {
                    let a = self.layer.contents(cell);
                    let b = other.layer.contents(cell);
                    let mut merged = if a.is_empty() {
                        self.implicit_cell_alternatives(cell, b, false)
                    } else {
                        a.to_vec()
                    };
                    if b.is_empty() {
                        merge_alternatives(
                            &mut merged,
                            other.implicit_cell_alternatives(cell, a, false),
                        );
                    } else {
                        merge_alternatives(&mut merged, b.iter().cloned());
                    }
                    (cell, merged)
                })
                .collect();
            changed |= self.layer.contents != contents;
            self.layer.replace_contents(contents);
        }

        // Mix the receiver and result only after checking detachment on each incoming path.
        let mut detached = self.layer.detached.clone();
        detached.extend(other.layer.detached.iter().copied());
        detached.retain(|pair| self.proves_detached(*pair) && other.proves_detached(*pair));
        changed |= detached != self.layer.detached;
        if detached != self.layer.detached {
            self.invalidate_detachments();
            let table = self.layer.table.borrow();
            for key in detached {
                AllocationLayer::edit_detachment(
                    &mut self.layer.detached,
                    &mut self.layer.detached_by_base,
                    &table,
                    key,
                    true,
                );
            }
        }

        // An overridden slot holds its ordinary contents on the opposite incoming path.
        // Equal groups need neither a fallback nor a merge.
        if self.layer.bound != other.layer.bound {
            let mut bound = self.layer.bound.clone();
            for (source, opposite) in [(&*self, other), (other, &*self)] {
                for &slot in source.layer.bound.keys() {
                    if opposite.owns_slot(slot) && opposite.layer.bound_of(slot).is_empty() {
                        bound.entry(slot).or_default().extend(
                            opposite
                                .slot_values(slot)
                                .map(|value| (slot, value.id, !value.writable)),
                        );
                    }
                }
            }
            for (&slot, group) in &other.layer.bound {
                bound.entry(slot).or_default().extend_from_slice(group);
            }
            changed |= self.layer.replace_bound(bound);
        }

        // A group explicit on one side holds the lazy child on the other.
        if self.layer.edges.groups != other.layer.edges.groups {
            let mut table = self.layer.table.borrow_mut();
            let mine = &mut self.layer.edges;
            let theirs = &other.layer.edges;
            // Lazy children are interned in predecessor order, not the merged group's
            // order: their allocation IDs can break ties in diagnostic attribution.
            for (source, opposite) in [(&*mine, theirs), (theirs, &*mine)] {
                for &(holder, key) in source.groups.keys() {
                    if opposite.group(holder, key).is_empty() {
                        table.child(holder, key);
                    }
                }
            }
            let groups: BTreeSet<_> = mine
                .groups
                .keys()
                .chain(theirs.groups.keys())
                .copied()
                .collect();
            for (holder, key) in groups {
                let left = mine.group(holder, key);
                let right = theirs.group(holder, key);
                if left == right {
                    continue;
                }
                if left.is_empty() || right.is_empty() {
                    let lazy = table.child(holder, key);
                    changed |= mine.insert(
                        &table,
                        Edge {
                            holder,
                            key,
                            held: lazy,
                            writable: true,
                            single: false,
                        },
                        false,
                    );
                }
                for &edge in right {
                    changed |= mine.insert(&table, edge, false);
                }
            }
        }

        changed |= self
            .layer
            .join_observations(&other.layer, |entry, incoming| {
                let old = entry.clone();
                let had_views = !entry.views.is_empty();
                if !had_views && !incoming.views.is_empty() {
                    entry
                        .views
                        .extend(entry.values.iter().map(|value| value.id));
                }
                if had_views && incoming.views.is_empty() {
                    entry
                        .views
                        .extend(incoming.values.iter().map(|value| value.id));
                }
                if !entry.alternatives.is_empty() || !incoming.alternatives.is_empty() {
                    if entry.alternatives.is_empty() {
                        entry.alternatives = compact_alternatives(entry.values.iter().copied());
                    }
                    if incoming.alternatives.is_empty() {
                        merge_alternatives(
                            &mut entry.alternatives,
                            compact_alternatives(incoming.values.iter().copied()),
                        );
                    } else {
                        merge_alternatives(
                            &mut entry.alternatives,
                            incoming.alternatives.iter().cloned(),
                        );
                    }
                }
                entry.views.extend_from_slice(&incoming.views);
                entry.values.extend_from_slice(&incoming.values);
                entry.readonly |= incoming.readonly;
                normalize_observation(entry);
                *entry != old
            });
        changed
    }

    fn owns_slot(&self, wrapper: WrapperId) -> bool {
        match self.binding_row(wrapper) {
            Some(row) => row.has_slot(),
            None => {
                wrapper.age == WrapperAge::Current && self.locals[wrapper.local].init != Init::No
            }
        }
    }

    fn merged_wrappers(&self, other: &Self) -> Option<Vec<BindingRow>> {
        let left = &self.layer.wrappers;
        let right = &other.layer.wrappers;
        if left == right {
            return None;
        }
        let mut rows = Vec::with_capacity(left.len().max(right.len()));
        let mut mine = left.iter().peekable();
        let mut theirs = right.iter().peekable();
        while let Some(wrapper) = mine
            .peek()
            .map(|row| row.wrapper)
            .into_iter()
            .chain(theirs.peek().map(|row| row.wrapper))
            .min()
        {
            let a = mine.next_if(|row| row.wrapper == wrapper);
            let b = theirs.next_if(|row| row.wrapper == wrapper);
            let mut row = a.or(b).expect("merge key belongs to one input").clone();
            if let (Some(_), Some(b)) = (a, b) {
                row.union_row_alts(b);
                row.route_roots.union_with(&b.route_roots);
                if b.coverage == Coverage::Partial {
                    row.coverage = Coverage::Partial;
                }
            } else if (a.is_none() && self.owns_slot(wrapper))
                || (b.is_none() && other.owns_slot(wrapper))
            {
                row.union_alts(&[OWN_SLOT]);
            }
            if row.has_slot()
                && (wrapper.age == WrapperAge::Prior
                    || row.alts.as_slice().len() > 1
                    || row.coverage == Coverage::Partial)
            {
                let mut inputs = [self, other]
                    .into_iter()
                    .filter(|state| state.owns_slot(wrapper))
                    .map(|state| state.slot_roots(wrapper));
                let mut roots = inputs
                    .next()
                    .expect("owned slot has an incoming occurrence")
                    .clone();
                if let Some(incoming) = inputs.next() {
                    if roots.is_empty() != incoming.is_empty() {
                        roots.insert(wrapper.local);
                    }
                    roots.union_with(incoming);
                }
                row.slot_roots = Some(roots);
            }
            rows.push(row);
        }
        (self.layer.wrappers != rows).then_some(rows)
    }

    fn join_sharing(&mut self, other: &Self) -> bool {
        let mine = &self.layer.sharing;
        let theirs = &other.layer.sharing;
        if mine.is_empty() && theirs.is_empty() {
            return false;
        }
        let mut sharing = SharingLayer::default();
        for (source, opposite, opposite_state) in [(mine, theirs, other), (theirs, mine, &*self)] {
            for &(view, id) in &source.members {
                sharing.insert_member(view, id);
            }
            sharing.bindings.extend_from_slice(&source.bindings);
            let mut previous = None;
            for &(slot, _) in &source.bindings {
                if previous.replace(slot) != Some(slot)
                    && opposite.bindings(slot).next().is_none()
                    && opposite_state.owns_slot(slot)
                {
                    sharing.bindings.extend(
                        opposite_state
                            .slot_values(slot)
                            .map(|value| (slot, value.id)),
                    );
                }
            }
            let mut table = self.layer.table.borrow_mut();
            for (&(holder, key), children) in &source.projections {
                let group = sharing.projections.entry((holder, key)).or_default();
                group.extend(children);
                if opposite.projections(holder, key).next().is_some()
                    || (matches!(table.base(holder), AllocBase::CallView(..))
                        && opposite.members(holder).next().is_none())
                {
                    continue;
                }
                let mut ids = Vec::new();
                opposite_state
                    .layer
                    .project_members(&mut table, holder, key, &mut ids);
                group.extend(ids);
            }
            for &(a, b, shares) in source.pairs.iter() {
                let could_exist = |view| {
                    !matches!(table.base(view), AllocBase::CallView(..))
                        || opposite.members(view).next().is_some()
                };
                let possible = shares
                    || (could_exist(a)
                        && could_exist(b)
                        && opposite_state.layer.views_may_share(a, b));
                sharing.pairs.union(a, b, possible);
            }
        }
        sharing.bindings.sort_unstable();
        sharing.bindings.dedup();
        let changed = &sharing != mine;
        sharing.rebuild_bases(&self.layer.table.borrow());
        self.layer.sharing = sharing;
        changed
    }

    /// Keep departing bindings' allocations named and drop facts no visible binding reaches.
    ///
    /// WHY: runs before the locals are narrowed. A visible binding whose roots name a departing
    ///      binding keeps that allocation as an override, and every id rooted at the departing
    ///      binding moves to a base that outlives the name.
    pub(super) fn narrow_layer(&mut self, visible: &RootSet) {
        let retires = |local| !visible.contains(local);
        if !self.layer.observations.is_empty() {
            let retired: BTreeSet<_> = self
                .layer
                .wrapper_users
                .iter()
                .map(|&(target, _)| target)
                .filter(|target| target.age == WrapperAge::Current && retires(target.local))
                .chain(
                    self.layer
                        .observations
                        .keys()
                        .copied()
                        .filter(|&local| retires(local))
                        .map(WrapperId::current),
                )
                .collect();
            self.invalidate_observation_routes(retired);
        }
        // First capture all departing slot payloads in the pre-edge allocation coordinates.
        if !self.layer.wrappers.is_empty() {
            let reached = self.reachable_wrappers(|local| visible.contains(local));
            let mut reached: Vec<_> = reached
                .into_iter()
                .filter(|wrapper| {
                    wrapper.age == WrapperAge::Current
                        && retires(wrapper.local)
                        && self.locals[wrapper.local].init != Init::No
                })
                .collect();
            // Keep allocation insertion independent of hash iteration.
            reached.sort_unstable();
            for wrapper in reached {
                self.materialize_prior(wrapper);
            }
        }
        let observers = self
            .value_root_users
            .iter()
            .chain(&self.layer.slot_root_users)
            .filter_map(|&(root, observer)| {
                increment_frontend_counter(FrontendCounter::BorrowRetirementLocalRows);
                (retires(root) && visible.contains(observer)).then_some(observer)
            })
            .collect::<BTreeSet<_>>();
        self.freeze_slot_observers(observers.into_iter(), |roots| {
            roots.has_members_outside(visible)
        });
        if self.layer.is_empty() {
            return;
        }
        self.layer
            .rename_bases(None, |base| departed_base(base, visible));
        let retired: BTreeSet<_> = self
            .layer
            .wrapper_users
            .iter()
            .filter(|(target, _)| target.age == WrapperAge::Current)
            .map(|(target, _)| target.local)
            .filter(|&local| retires(local))
            .collect();
        self.age_wrappers(retired, retires);
        self.prune_wrappers(|local| visible.contains(local));
        for row in &mut self.layer.wrappers {
            row.route_roots.intersect_with(visible);
            let tracks_slot = row.has_slot() && row.wrapper.age == WrapperAge::Current;
            if let Some(roots) = &mut row.slot_roots {
                if tracks_slot {
                    super::index_roots(
                        &mut self.layer.slot_root_users,
                        row.wrapper.local,
                        roots,
                        false,
                    );
                }
                roots.intersect_with(visible);
                if tracks_slot {
                    super::index_roots(
                        &mut self.layer.slot_root_users,
                        row.wrapper.local,
                        roots,
                        true,
                    );
                }
            }
        }
        self.rebuild_observation_routes();
        self.layer
            .retain_observations(|entry| visible.contains(entry.local));
        self.layer
            .retain_detachment_owners(|owner| visible.contains(owner as usize));
        self.collect_unreachable_allocations(visible);
    }

    fn reachable_wrappers(&self, live_name: impl Fn(usize) -> bool) -> FxHashSet<WrapperId> {
        self.reachable_from(
            self.layer
                .wrappers
                .iter()
                .filter_map(|row| {
                    (row.wrapper.age == WrapperAge::Current && live_name(row.wrapper.local))
                        .then_some(row.wrapper)
                })
                .collect(),
        )
    }

    fn reachable_from(&self, mut pending: Vec<WrapperId>) -> FxHashSet<WrapperId> {
        let mut reached = FxHashSet::default();
        while let Some(wrapper) = pending.pop() {
            if reached.insert(wrapper)
                && let Some(row) = self.binding_row(wrapper)
            {
                pending.extend(
                    row.alts
                        .as_slice()
                        .iter()
                        .filter_map(|alt| match alt.target {
                            BindingTarget::Link(source) => Some(source),
                            _ => None,
                        }),
                );
            }
        }
        reached
    }

    fn prune_wrappers(&mut self, live_name: impl Fn(usize) -> bool) {
        let reached = self.reachable_wrappers(&live_name);
        let keep = |wrapper: WrapperId| match wrapper.age {
            WrapperAge::Current => live_name(wrapper.local),
            WrapperAge::Prior => reached.contains(&wrapper),
        };
        self.layer.retain_wrappers(keep);
        self.layer.retain_bound(keep);
        self.layer.retain_contents(|cell, _| match cell {
            CellId::Slot(slot) => keep(slot),
            CellId::Projected(..) => true,
        });
        self.layer
            .sharing
            .retain_binding_owners(&self.layer.table.borrow(), keep);
    }

    pub(in super::super) fn has_retired_wrappers(&self) -> bool {
        self.layer
            .wrappers
            .iter()
            .any(|row| row.wrapper.age == WrapperAge::Prior)
    }

    pub(in super::super) fn prune_retired_wrappers(&mut self) {
        if self.has_retired_wrappers() {
            self.prune_wrappers(|_| true);
        }
    }

    /// Drop edges whose holder no visible binding reaches.
    fn collect_unreachable_allocations(&mut self, visible: &super::RootSet) {
        if self.layer.edges.is_empty()
            && self.layer.sharing.is_empty()
            && self.layer.contents.is_empty()
        {
            return;
        }
        let mut live: Vec<AllocId> = Vec::new();
        for local in visible.iter_ones() {
            if self.locals[local].init == Init::No {
                continue;
            }
            let slot = WrapperId::current(local);
            if self.binding_row(slot).is_none()
                && self.layer.bound_of(slot).is_empty()
                && self.layer.observation(local).is_none()
            {
                // An ordinary slot can seed the graph without resolving addresses or views.
                let roots = &self.locals[local].value_roots;
                let mut table = self.layer.table.borrow_mut();
                if roots.is_empty() {
                    live.push(table.root(local));
                } else {
                    live.extend(roots.iter_ones().map(|root| table.root(root)));
                }
            } else {
                live.extend(self.local_alloc_ids(local).0);
            }
        }
        live.extend(self.layer.projected_holders());
        live.extend(
            self.layer
                .bound
                .values()
                .flatten()
                .filter_map(|&(_, id, _)| (id != AllocId::EMPTY).then_some(id)),
        );
        let table = self.layer.table.borrow();
        let mut reached: FxHashSet<AllocId> = live.iter().copied().collect();
        let mut stack = live;
        let mut kept_holders = FxHashSet::default();
        while let Some(current) = stack.pop() {
            increment_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows);
            if !kept_holders.insert(current) {
                continue;
            }
            stack.extend(self.layer.edges.children_of(current));
            for edge in self.layer.edges.for_holder(current) {
                increment_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows);
                if edge.held != AllocId::EMPTY && reached.insert(edge.held) {
                    stack.push(edge.held);
                }
            }
        }
        let has_reached_ancestor = |table: &AllocTable, mut holder| {
            loop {
                increment_frontend_counter(FrontendCounter::BorrowRetirementAllocationRows);
                if reached.contains(&holder) {
                    return true;
                }
                let Some((parent, _)) = table.parent(holder) else {
                    return false;
                };
                holder = parent;
            }
        };
        if !self.layer.sharing.is_empty() {
            let sharing = &mut self.layer.sharing;
            let mut live_views = FxHashSet::default();
            let mut visit: Vec<AllocId> = sharing.bindings.iter().map(|&(_, view)| view).collect();
            for &(holder, _) in sharing.projections.keys() {
                if has_reached_ancestor(&table, holder) {
                    visit.push(holder);
                }
            }
            while let Some(view) = visit.pop() {
                if !live_views.insert(view) {
                    continue;
                }
                visit.extend(sharing.projected_by(view).map(|(_, child)| child));
            }
            sharing.retain_live_views(&table, &live_views);
        }
        drop(table);
        self.layer.retain_contents(|cell, table| match cell {
            CellId::Projected(holder, _) => has_reached_ancestor(table, holder),
            CellId::Slot(_) => true,
        });
        let discarded: Vec<_> = self
            .layer
            .edges
            .groups
            .keys()
            .copied()
            .filter(|(holder, _)| !kept_holders.contains(holder))
            .collect();
        let table = self.layer.table.borrow();
        for (holder, key) in discarded {
            self.layer.edges.take_group(&table, holder, key);
        }
    }
}

fn departed_base(base: AllocBase, visible: &RootSet) -> Option<AllocBase> {
    match base {
        AllocBase::Root(local) if !visible.contains(local as usize) => {
            Some(AllocBase::Departed(local))
        }
        _ => None,
    }
}

fn base_local(base: AllocBase) -> Option<usize> {
    match base {
        AllocBase::Root(local) | AllocBase::Old(local, _) | AllocBase::Departed(local) => {
            Some(local as usize)
        }
        AllocBase::Site(_)
        | AllocBase::Call(..)
        | AllocBase::OpaqueCall(..)
        | AllocBase::CallView(..) => None,
    }
}

/// Blocks that can reach themselves.
pub(in super::super) fn cyclic_blocks(
    successors: &FxHashMap<BlockId, Vec<BlockId>>,
) -> FxHashSet<BlockId> {
    // Iterative Tarjan over the reachable successor map.
    let mut index_of: FxHashMap<BlockId, (u32, u32)> = FxHashMap::default();
    let mut on_stack: FxHashSet<BlockId> = FxHashSet::default();
    let mut stack: Vec<BlockId> = Vec::new();
    let mut cyclic = FxHashSet::default();
    let mut next_index = 0u32;
    let mut roots: Vec<BlockId> = successors.keys().copied().collect();
    roots.sort_unstable_by_key(|block| block.0);
    let empty = Vec::new();
    for root in roots {
        if index_of.contains_key(&root) {
            continue;
        }
        let mut work: Vec<(BlockId, usize)> = vec![(root, 0)];
        index_of.insert(root, (next_index, next_index));
        next_index += 1;
        stack.push(root);
        on_stack.insert(root);
        while let Some((block, child)) = work.pop() {
            let next = successors.get(&block).unwrap_or(&empty);
            if let Some(successor) = next.get(child).copied() {
                work.push((block, child + 1));
                if successor == block {
                    cyclic.insert(block);
                }
                match index_of.get(&successor).copied() {
                    None => {
                        index_of.insert(successor, (next_index, next_index));
                        next_index += 1;
                        stack.push(successor);
                        on_stack.insert(successor);
                        work.push((successor, 0));
                    }
                    Some((successor_index, _)) if on_stack.contains(&successor) => {
                        let entry = index_of.get_mut(&block).expect("visited block");
                        entry.1 = entry.1.min(successor_index);
                    }
                    Some(_) => {}
                }
                continue;
            }
            let (block_index, block_low) = index_of[&block];
            if let Some((parent, _)) = work.last() {
                let parent = *parent;
                let entry = index_of.get_mut(&parent).expect("visited block");
                entry.1 = entry.1.min(block_low);
            }
            if block_low == block_index {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack.remove(&member);
                    component.push(member);
                    if member == block {
                        break;
                    }
                }
                if component.len() > 1 {
                    cyclic.extend(component);
                }
            }
        }
    }
    cyclic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_interns_missing_children_in_predecessor_order_for_readonly_attribution() {
        for preinterned in [false, true] {
            for left_is_higher in [false, true] {
                let mut base = BorrowState::new_uninitialized(3);
                for local in 0..3 {
                    base.initialize_parameter(local);
                }
                let [low, high, result] = [0, 1, 2].map(|local| base.local_alloc_ids(local).0[0]);
                let (left, right) = if left_is_higher {
                    (high, low)
                } else {
                    (low, high)
                };
                let field = FieldKey::Field(0);
                if preinterned {
                    base.alloc_child(right, field);
                    base.alloc_child(left, field);
                }
                let mut first = base.clone();
                let mut second = base;
                let member = StoredValue {
                    id: result,
                    writable: true,
                };
                first.add_alloc_edge(left, field, member);
                second.add_alloc_edge(right, field, member);
                first.join_in_place(&second);
                let a = first.alloc_child(left, field);
                let b = first.alloc_child(right, field);
                assert_eq!(
                    a < b,
                    !preinterned,
                    "fresh children must follow predecessor order"
                );
                first.add_alloc_edge(
                    result,
                    field,
                    StoredValue {
                        id: a,
                        writable: false,
                    },
                );
                first.add_alloc_edge(
                    result,
                    field,
                    StoredValue {
                        id: b,
                        writable: false,
                    },
                );
                let mut resolved = first.resolve_local_allocs(2);
                first.step_resolved(&mut resolved, field, &[false, false, true]);
                let expected_origin = usize::from(left_is_higher != preinterned);
                assert_eq!(resolved.readonly_source, Some(expected_origin));
            }
        }
    }

    #[test]
    fn projected_write_selects_only_observations_on_the_same_selector() {
        let mut state = BorrowState::new_uninitialized(65);
        state.initialize_parameter(0);
        let holder = state.local_alloc_ids(0).0[0];
        for local in 1..65 {
            let cell = CellId::Projected(holder, FieldKey::Field(local as _));
            state.initialize_entry_binding(
                local,
                BindingSpec::Projected {
                    addresses: AddressSet::from_cells([cell], Coverage::Complete),
                    origin: WrapperId::current(0),
                    owner: None,
                    writable: true,
                    isolated: true,
                    route_roots: RootSet::empty(65),
                },
            );
            state.record_observation(Observation {
                local,
                values: Vec::new(),
                views: Vec::new(),
                alternatives: Vec::new(),
                readonly: false,
            });
        }
        let cell = CellId::Projected(holder, FieldKey::Field(33));
        assert_eq!(state.observation_candidates(&[cell]), BTreeSet::from([33]));
        state.invalidate_observations(&[cell], Coverage::Complete);
        assert_eq!(state.layer.observations.len(), 63);
        assert!(!state.layer.observations.contains_key(&33));
        state.assert_allocation_incidence();
    }

    #[test]
    fn retirement_moves_only_rows_incident_on_the_old_base() {
        let mut state = BorrowState::new_uninitialized(3);
        for local in 0..3 {
            state.initialize_parameter(local);
        }
        let [old, survivor, other] = [0, 1, 2].map(|local| state.local_alloc_ids(local).0[0]);
        let view = state.call_view_id(1, false, 0);
        let field = FieldKey::Field(0);
        state.add_view_member(view, old);
        state.add_view_member(view, survivor);
        state.relate_views(old, survivor, true);
        state.link_view(view, field, old);
        state.link_view(view, FieldKey::Field(1), other);
        state.bind_detached_holders(2, [old, survivor]);
        state.bind_alloc_ids(1, &[old, other], false);
        state.record_observation(Observation {
            local: 2,
            values: Vec::new(),
            views: Vec::new(),
            alternatives: vec![ValueAlt {
                kind: ValueKind::Scalar,
                deps: vec![super::super::ReceiverDependency {
                    entries: SmallSet::from_iter([old]),
                    coverage: Coverage::Complete,
                }],
            }],
            readonly: true,
        });
        state.assert_allocation_incidence();

        state.retire_slot_value(0, 2);
        let rebased = state.remap_retired_alloc_id(old, 2, |local| local == 0);
        assert_ne!(old, rebased);
        assert!(state.layer.sharing.members(view).any(|id| id == rebased));
        assert!(state.layer.sharing.members(view).any(|id| id == survivor));
        assert_eq!(
            state.layer.sharing.possible_pair(rebased, survivor),
            Some(true)
        );
        assert_eq!(
            state.layer.sharing.projections(view, field).next(),
            Some(rebased)
        );
        assert_eq!(
            state
                .layer
                .sharing
                .projections(view, FieldKey::Field(1))
                .next(),
            Some(other)
        );
        assert_eq!(state.detached_holders(2).collect::<Vec<_>>(), [survivor]);
        assert_eq!(
            state.layer.observation(2).unwrap().alternatives[0].deps[0]
                .entries
                .as_slice(),
            &[rebased],
        );
        assert!(
            state
                .layer
                .bound_of(WrapperId::current(1))
                .iter()
                .any(|&(_, id, _)| id == rebased)
        );
        state.assert_allocation_incidence();

        state.bind_slot_views(WrapperId::current(1), &[view]);
        let mut joined = state.clone();
        joined.join_in_place(&state);
        joined.assert_allocation_incidence();
        let mut visible = RootSet::empty(3);
        visible.insert(1);
        joined.kill_invisible(&visible);
        assert!(joined.layer.observation(2).is_none());
        assert!(joined.detached_holders(2).next().is_none());
        assert!(joined.layer.sharing.members(view).any(|id| id == rebased));
        joined.assert_allocation_incidence();
    }

    #[test]
    fn implicit_scalar_definition_and_store_need_no_sparse_storage() {
        let mut state = BorrowState::new_uninitialized(1);
        state.rebind_local(0, LocalState::slot(1), 0);
        let addresses = state.resolve_cells(0);
        assert_eq!(
            addresses.definite(),
            Some(CellId::Slot(WrapperId::current(0)))
        );
        let write = TypedStore::replace_cells(addresses, Default::default())
            .with_slot_roots(RootSet::empty(1));
        state.publish_store(&write, 1, None);
        assert_eq!(state.layer.wrappers.capacity(), 0);
        assert!(state.layer.bound.is_empty());
        assert!(state.layer.table.borrow().infos.is_empty());
    }

    #[test]
    fn boundary_observes_the_immediate_link_and_the_resolved_cell_separately() {
        let mut state = BorrowState::new_uninitialized(3);
        for local in 0..3 {
            state.rebind_local(local, LocalState::slot(3), 0);
        }
        let source = WrapperId::current(0);
        let intermediate = WrapperId::current(1);
        let alias = WrapperId::current(2);
        state.install_binding_spec(
            1,
            BindingSpec::Link {
                source,
                writable: true,
            },
        );
        state.install_binding_spec(
            2,
            BindingSpec::Link {
                source: intermediate,
                writable: false,
            },
        );

        let binding = state.boundary_binding(alias);
        assert_eq!(binding.coverage, Coverage::Complete);
        assert_eq!(
            binding.alternatives[0].target,
            BindingTarget::Link(intermediate)
        );
        assert!(!binding.alternatives[0].writable);
        assert_eq!(state.boundary_binding(source).alternatives, &[OWN_SLOT]);
        let place = state.resolve_place_path(2, &[true; 3], []);
        assert_eq!(place.address.definite(), Some(CellId::Slot(source)));
        assert!(place.content.is_none());
        assert_eq!(place.permission.wrapper_readonly_source, Some(2));
        assert!(state.layer.table.borrow().infos.is_empty());
    }

    #[test]
    fn boundary_keeps_implicit_and_explicit_slot_alternatives_distinct() {
        let mut state = BorrowState::new_uninitialized(2);
        let slot = WrapperId::current(0);
        assert!(state.boundary_binding(slot).alternatives.is_empty());
        assert_eq!(state.boundary_binding(slot).coverage, Coverage::Partial);
        state.rebind_local(0, LocalState::slot(2), 0);
        assert_eq!(state.boundary_binding(slot).coverage, Coverage::Complete);
        assert!(state.retained_slot_payload(slot).is_none());

        state
            .layer
            .set_bound(slot, vec![(slot, AllocId::EMPTY, false)]);
        assert_eq!(
            state
                .retained_slot_payload(slot)
                .unwrap()
                .collect::<Vec<_>>(),
            [ValueKind::Absent],
        );
        let allocation = state.layer.table.borrow_mut().root(0);
        state.layer.extend_bound(slot, [(slot, allocation, true)]);
        assert_eq!(
            state
                .retained_slot_payload(slot)
                .unwrap()
                .collect::<Vec<_>>(),
            [
                ValueKind::Allocation(StoredValue {
                    id: allocation,
                    writable: false
                }),
                ValueKind::Absent
            ],
        );
        assert_eq!(state.layer.table.borrow().infos.len(), 1);
        state.assert_allocation_incidence();
    }
}
