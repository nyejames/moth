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
    AddressSet, BindingAlt, BindingRow, BindingSpec, BindingTarget, BindingView, BorrowState,
    CellId, Coverage, EntryFamilies, Init, LocalState, OWN_SLOT, OrderKey, Origin, Replacement,
    RootSet, SmallSet, TypedStore, TypedWrite, WrapperAge, WrapperId, remap_invoke_capture,
};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::ids::BlockId;
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in super::super) struct AllocId(u32);

impl AllocId {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Default)]
pub(in super::super) struct AllocTable {
    infos: Vec<AllocInfo>,
    index: FxHashMap<AllocKey, AllocId>,
    cyclic_blocks: Option<FxHashSet<BlockId>>,
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
        self.cyclic_blocks
            .get_or_insert_with(|| cyclic_blocks(successors))
            .contains(&block)
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
struct Edge {
    holder: AllocId,
    key: FieldKey,
    held: AllocId,
    /// Whether a mutation may pass through this edge. A value stored from an immutable binding
    /// keeps that binding's permission.
    writable: bool,
    /// Whether this group holds this allocation at most once.
    single: bool,
}

impl Edge {
    fn sort_key(&self) -> (AllocId, FieldKey, AllocId) {
        (self.holder, self.key, self.held)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Observation {
    local: usize,
    values: Vec<StoredValue>,
    views: Vec<AllocId>,
    readonly: bool,
}

struct ResolvedCells {
    addresses: AddressSet,
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
        for &cell in suffix.addresses.cells() {
            self.addresses.cells.insert(cell);
        }
        self.readonly_source = self.readonly_source.or(suffix.readonly_source);
        self.cyclic |= suffix.cyclic;
        self.prior |= suffix.prior;
    }
}

/// A view names an occurrence, not a new allocation. Its members give ordinary provenance,
/// while a projection can name a new occurrence even when both paths have the same members.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SharingLayer {
    members: Vec<(AllocId, AllocId)>,
    projections: Vec<(AllocId, FieldKey, AllocId)>,
    bindings: Vec<(WrapperId, AllocId)>,
    pairs: Vec<(AllocId, AllocId, bool)>,
}

impl SharingLayer {
    fn members(&self, view: AllocId) -> impl Iterator<Item = AllocId> + '_ {
        let start = self.members.partition_point(|(key, _)| *key < view);
        self.members[start..]
            .iter()
            .take_while(move |(key, _)| *key == view)
            .map(|(_, id)| *id)
    }

    fn projections(&self, view: AllocId, key: FieldKey) -> impl Iterator<Item = AllocId> + '_ {
        let start = self
            .projections
            .partition_point(|&(holder, field, _)| (holder, field) < (view, key));
        self.projections[start..]
            .iter()
            .take_while(move |&&(holder, field, _)| (holder, field) == (view, key))
            .map(|&(_, _, child)| child)
    }

    fn bindings(&self, local: WrapperId) -> impl Iterator<Item = AllocId> + '_ {
        let start = self.bindings.partition_point(|(key, _)| *key < local);
        self.bindings[start..]
            .iter()
            .take_while(move |(key, _)| *key == local)
            .map(|(_, view)| *view)
    }

    fn possible_pair(&self, left: AllocId, right: AllocId) -> Option<bool> {
        let (left, right) = ordered_pair(left, right);
        self.pairs
            .binary_search_by_key(&(left, right), |&(a, b, _)| (a, b))
            .ok()
            .map(|index| self.pairs[index].2)
    }

    fn union_pair(&mut self, left: AllocId, right: AllocId, may_share: bool) {
        let (left, right) = ordered_pair(left, right);
        match self
            .pairs
            .binary_search_by_key(&(left, right), |&(a, b, _)| (a, b))
        {
            Ok(index) => self.pairs[index].2 |= may_share,
            Err(index) => self.pairs.insert(index, (left, right, may_share)),
        }
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

/// Explicit heap and slot contents, plus the sparse wrapper relation.
#[derive(Debug, Clone)]
pub(in super::super) struct AllocationLayer {
    table: Rc<RefCell<AllocTable>>,
    edges: Vec<Edge>,
    /// A missing current slot payload is described by its ordinary value roots.
    bound: Vec<(WrapperId, AllocId, bool)>,
    wrappers: Vec<BindingRow>,
    observations: Vec<Observation>,
    /// (Result binding, former holder) pairs whose reachable graphs are disjoint on every path.
    detached: Vec<(u32, AllocId)>,
    /// (Lookup result, receiver) loans protect collection structure independently of values.
    stability: Vec<(u32, AllocId)>,
    sharing: SharingLayer,
}

impl PartialEq for AllocationLayer {
    fn eq(&self, other: &Self) -> bool {
        self.edges == other.edges
            && self.bound == other.bound
            && self.wrappers == other.wrappers
            && self.observations == other.observations
            && self.detached == other.detached
            && self.stability == other.stability
            && self.sharing == other.sharing
    }
}

impl Eq for AllocationLayer {}

impl AllocationLayer {
    pub(super) fn new() -> Self {
        Self {
            table: Rc::new(RefCell::new(AllocTable::default())),
            edges: Vec::new(),
            bound: Vec::new(),
            wrappers: Vec::new(),
            observations: Vec::new(),
            detached: Vec::new(),
            stability: Vec::new(),
            sharing: SharingLayer::default(),
        }
    }

    fn is_empty(&self) -> bool {
        self.edges.is_empty()
            && self.bound.is_empty()
            && self.wrappers.is_empty()
            && self.observations.is_empty()
            && self.detached.is_empty()
            && self.stability.is_empty()
            && self.sharing.is_empty()
    }

    fn group(&self, holder: AllocId, key: FieldKey) -> &[Edge] {
        let start = self
            .edges
            .partition_point(|edge| (edge.holder, edge.key) < (holder, key));
        let end = start
            + self.edges[start..]
                .iter()
                .take_while(|edge| edge.holder == holder && edge.key == key)
                .count();
        &self.edges[start..end]
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
        let group = self.group(holder, key);
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
        let start = self.bound.partition_point(|(owner, _, _)| *owner < local);
        let end = start
            + self.bound[start..]
                .iter()
                .take_while(|(owner, _, _)| *owner == local)
                .count();
        &self.bound[start..end]
    }

    fn observation(&self, local: usize) -> Option<&Observation> {
        self.observations
            .binary_search_by_key(&local, |entry| entry.local)
            .ok()
            .map(|index| &self.observations[index])
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
        match self
            .edges
            .binary_search_by(|probe| probe.sort_key().cmp(&edge.sort_key()))
        {
            Ok(position) => {
                let existing = &mut self.edges[position];
                existing.writable &= edge.writable;
                // Two stores may leave a duplicate behind after removal. A copied
                // graph fact instead describes the same already-counted occurrences.
                existing.single &= edge.single && !distinct_occurrence;
            }
            Err(position) => self.edges.insert(position, edge),
        }
    }

    /// Whether `descendant` is `ancestor` or a lazy descendant still held through every step.
    fn covers(&self, table: &AllocTable, ancestor: AllocId, descendant: AllocId) -> bool {
        let mut current = descendant;
        loop {
            if current == ancestor {
                return true;
            }
            let Some((parent, key)) = table.parent(current) else {
                return false;
            };
            let group = self.group(parent, key);
            if !group.is_empty() && !group.iter().any(|edge| edge.held == current) {
                return false;
            }
            current = parent;
        }
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
        if !self.group(parent, key).is_empty() {
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
            .filter_map(|(parent, key)| self.group(parent, key).is_empty().then_some(parent))
            .collect();
        parents.sort_unstable();
        parents.dedup();
        for parent in parents {
            self.copied_path(table, &mut copies, site, parent);
        }
        for (&source, &held) in &copies {
            if let Some((parent, key)) = table.parent(source)
                && let Some(&holder) = copies.get(&parent)
                && self.group(parent, key).is_empty()
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
            .iter()
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
                for edge in &self.edges {
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
                let start = self
                    .sharing
                    .projections
                    .partition_point(|(holder, _, _)| *holder < view);
                keys.extend(
                    self.sharing.projections[start..]
                        .iter()
                        .take_while(|(holder, _, _)| *holder == view)
                        .map(|(_, key, _)| *key),
                );
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

    /// Renumber every id whose base is `from` onto `to`.
    fn rename_base(&mut self, from: AllocBase, to: AllocBase, to_singular: bool) {
        let mut table = self.table.borrow_mut();
        let mentions = |id: AllocId| id != AllocId::EMPTY && table.base(id) == from;
        self.detached.retain(|(_, holder)| !mentions(*holder));
        let touched = self
            .edges
            .iter()
            .any(|edge| mentions(edge.holder) || mentions(edge.held))
            || self.bound.iter().any(|(_, id, _)| mentions(*id))
            || self.projected_holders().any(mentions)
            || self.observations.iter().any(|entry| {
                entry.values.iter().any(|value| mentions(value.id))
                    || entry.views.iter().copied().any(mentions)
            })
            || self.stability.iter().any(|&(_, holder)| mentions(holder))
            || self
                .sharing
                .members
                .iter()
                .any(|&(view, id)| mentions(view) || mentions(id))
            || self
                .sharing
                .projections
                .iter()
                .any(|&(from, _, to)| mentions(from) || mentions(to))
            || self
                .sharing
                .bindings
                .iter()
                .any(|&(_, view)| mentions(view))
            || self
                .sharing
                .pairs
                .iter()
                .any(|&(left, right, _)| mentions(left) || mentions(right));
        if !touched {
            return;
        }
        let rename = |id: AllocId, table: &mut AllocTable| {
            if id != AllocId::EMPTY && table.base(id) == from {
                table.rebase(id, to, to_singular)
            } else {
                id
            }
        };
        for edge in &mut self.edges {
            edge.holder = rename(edge.holder, &mut table);
            edge.held = rename(edge.held, &mut table);
            if edge.held != AllocId::EMPTY && !table.singular(edge.held) {
                edge.single = false;
            }
        }
        self.edges.sort_by_key(Edge::sort_key);
        self.edges.dedup_by(|later, earlier| {
            if later.sort_key() != earlier.sort_key() {
                return false;
            }
            earlier.writable &= later.writable;
            earlier.single = false;
            true
        });
        for (_, id, _) in &mut self.bound {
            *id = rename(*id, &mut table);
        }
        normalize_bindings(&mut self.bound);
        for row in &mut self.wrappers {
            row.alts.remap(|alt| match alt.target {
                BindingTarget::Projected(holder, key) => BindingAlt {
                    target: BindingTarget::Projected(rename(holder, &mut table), key),
                    ..alt
                },
                _ => alt,
            });
        }
        for entry in &mut self.observations {
            for value in &mut entry.values {
                value.id = rename(value.id, &mut table);
            }
            for view in &mut entry.views {
                *view = rename(*view, &mut table);
            }
            normalize_observation(entry);
        }
        for (_, holder) in &mut self.stability {
            *holder = rename(*holder, &mut table);
        }
        self.stability.sort_unstable();
        self.stability.dedup();
        for (view, id) in &mut self.sharing.members {
            *view = rename(*view, &mut table);
            *id = rename(*id, &mut table);
        }
        self.sharing.members.sort_unstable();
        self.sharing.members.dedup();
        for (holder, _, child) in &mut self.sharing.projections {
            *holder = rename(*holder, &mut table);
            *child = rename(*child, &mut table);
        }
        self.sharing.projections.sort_unstable();
        self.sharing.projections.dedup();
        for (_, view) in &mut self.sharing.bindings {
            *view = rename(*view, &mut table);
        }
        self.sharing.bindings.sort_unstable();
        self.sharing.bindings.dedup();
        for (left, right, _) in &mut self.sharing.pairs {
            (*left, *right) = ordered_pair(rename(*left, &mut table), rename(*right, &mut table));
        }
        self.sharing.pairs.sort_unstable();
        self.sharing.pairs.dedup_by(|right, left| {
            if (left.0, left.1) != (right.0, right.1) {
                return false;
            }
            left.2 |= right.2;
            true
        });
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

/// Allocations a place reaches, with how the layer reached them.
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
    /// Every feasible location is represented, even if more than one allocation can occur.
    pub complete: bool,
}

/// One allocation stored into a holder field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A mutation target: the allocation and whether any alias reaching it conflicts, or only an
/// alias that reaches it through layer facts.
#[derive(Debug, Clone, Copy)]
pub(in super::super) struct MutationTarget {
    pub id: AllocId,
    pub layer_reached: bool,
    /// Whether the operation can also mutate fields of this allocation.
    pub deep: bool,
}

impl<'a> BindingView<'a> {
    pub(in super::super) fn projected_cells(self) -> impl Iterator<Item = (AllocId, FieldKey)> {
        let addresses = self.state.resolve_cells(self.local);
        let mut next = 0;
        std::iter::from_fn(move || {
            while let Some(cell) = addresses.cells().get(next) {
                next += 1;
                if let CellId::Projected(holder, key) = *cell {
                    return Some((holder, key));
                }
            }
            None
        })
    }

    pub(in super::super) fn projected_coverage(self) -> Coverage {
        let addresses = self.state.resolve_cells(self.local);
        if !addresses.cells().is_empty()
            && addresses
                .cells()
                .iter()
                .all(|cell| matches!(cell, CellId::Projected(..)))
        {
            addresses.coverage
        } else {
            Coverage::Partial
        }
    }

    pub(in super::super) fn projected_cells_share_singular_holder(self, holder: AllocId) -> bool {
        self.state.alloc_singular(holder)
            && self
                .projected_cells()
                .all(|(candidate, _)| candidate == holder)
    }

    pub(in super::super) fn projected_cells_touch(
        self,
        cells: &[(AllocId, FieldKey)],
        values: &[MutationTarget],
    ) -> bool {
        let table = self.state.layer.table.borrow();
        self.projected_cells().any(|(holder, key)| {
            cells.contains(&(holder, key))
                || values
                    .iter()
                    .any(|value| value.deep && self.state.layer.covers(&table, value.id, holder))
        })
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

    fn put_binding_row(&mut self, row: BindingRow) {
        match self
            .layer
            .wrappers
            .binary_search_by_key(&row.wrapper, |row| row.wrapper)
        {
            Ok(index) => self.layer.wrappers[index] = row,
            Err(index) => self.layer.wrappers.insert(index, row),
        }
    }

    pub(super) fn remove_current_row(&mut self, local: usize) {
        let wrapper = WrapperId::current(local);
        if let Ok(index) = self
            .layer
            .wrappers
            .binary_search_by_key(&wrapper, |row| row.wrapper)
        {
            self.layer.wrappers.remove(index);
        }
        self.layer.observations.retain(|entry| entry.local != local);
    }

    fn row_from_spec(&self, local: usize, spec: BindingSpec) -> Option<BindingRow> {
        let wrapper = WrapperId::current(local);
        let (alts, coverage, route_roots) = match spec {
            BindingSpec::Slot => return None,
            BindingSpec::Link { source, writable } => (
                SmallSet::One(BindingAlt {
                    target: BindingTarget::Link(source),
                    writable,
                    origin: None,
                }),
                Coverage::Complete,
                RootSet::empty(self.locals.len()),
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
            ),
            BindingSpec::Projected {
                addresses,
                origin,
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
                            isolated,
                        }),
                    }),
                    CellId::Slot(_) => None,
                })),
                addresses.coverage,
                route_roots,
            ),
        };
        Some(BindingRow {
            wrapper,
            alts,
            coverage,
            route_roots,
            slot_roots: None,
        })
    }

    pub(super) fn install_binding_spec(&mut self, local: usize, spec: BindingSpec) {
        if let Some(row) = self.row_from_spec(local, spec) {
            self.put_binding_row(row);
        }
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
            row.union_alts(old.alts.as_slice());
            row.route_roots.union_with(&old.route_roots);
            if old.coverage == Coverage::Partial {
                row.coverage = Coverage::Partial;
            }
            if self.binding(local).mode().contains(LocalMode::ALIAS) {
                self.locals[local]
                    .derived_from_roots
                    .intersect_with(&derivation);
            } else {
                self.locals[local].derived_from_roots = derivation;
            }
        } else {
            self.locals[local].derived_from_roots = derivation;
        }
        if preserve_slot {
            row.union_alts(&[OWN_SLOT]);
            row.slot_roots = Some(self.slot_roots(wrapper).clone());
            self.locals[local].value_roots.union_with(&roots);
        } else {
            row.alts = SmallSet::from_iter(
                row.alts
                    .as_slice()
                    .iter()
                    .copied()
                    .filter(|alt| alt.target != BindingTarget::OwnSlot),
            );
            self.clear_slot_payload(wrapper);
            self.locals[local].value_roots = roots;
        }
        self.put_binding_row(row);
    }

    pub(in super::super) fn mark_slot_derivation(&mut self, local: usize, roots: RootSet) {
        let mode = self.binding_mode(local);
        if mode.contains(LocalMode::SLOT) {
            if mode.contains(LocalMode::ALIAS) {
                self.locals[local].derived_from_roots.intersect_with(&roots);
            } else {
                self.locals[local].derived_from_roots = roots;
            }
        }
    }

    pub(in super::super) fn resolve_cells(&self, local: usize) -> AddressSet {
        self.resolve_wrapper(WrapperId::current(local), false, &[])
            .addresses
    }

    /// A mutable access needs every alternative and every wrapper along it to be writable.
    pub(in super::super) fn resolve_cells_with_permission(
        &self,
        local: usize,
        aliases_only: bool,
        local_mutable: &[bool],
    ) -> (AddressSet, Option<usize>) {
        let resolved = self.resolve_wrapper(WrapperId::current(local), aliases_only, local_mutable);
        (resolved.addresses, resolved.readonly_source)
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
        let init = if wrapper.age == WrapperAge::Current {
            self.locals[wrapper.local].init
        } else {
            Init::No
        };
        ResolvedCells {
            addresses: AddressSet::from_cells(
                (init != Init::No && !aliases_only).then_some(CellId::Slot(wrapper)),
                if init == Init::Yes {
                    Coverage::Complete
                } else {
                    Coverage::Partial
                },
            )
            .with_singleton_proof(),
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
        let partial = row.coverage == Coverage::Partial
            || wrapper.age == WrapperAge::Current && self.locals[wrapper.local].init != Init::Yes;
        ResolutionFrame {
            wrapper,
            alternatives: row.alts.as_slice().iter(),
            result: ResolvedCells {
                addresses: AddressSet::from_cells(
                    [],
                    if partial {
                        Coverage::Partial
                    } else {
                        Coverage::Complete
                    },
                ),
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

    fn projected_cell_is_exact(&self, holder: AllocId, key: FieldKey) -> bool {
        matches!(key, FieldKey::Field(_) | FieldKey::Payload { .. }) && self.alloc_singular(holder)
    }

    fn slot_roots(&self, wrapper: WrapperId) -> &RootSet {
        self.binding_row(wrapper)
            .and_then(|row| row.slot_roots.as_ref())
            .unwrap_or(&self.locals[wrapper.local].value_roots)
    }

    pub(in super::super) fn layer_is_empty(&self) -> bool {
        self.layer.is_empty()
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
        insert_fact(&mut self.layer.sharing.members, (view, id));
    }

    pub(in super::super) fn relate_views(&mut self, a: AllocId, b: AllocId, may_share: bool) {
        self.layer.sharing.union_pair(a, b, may_share);
    }

    pub(in super::super) fn link_view(&mut self, holder: AllocId, key: FieldKey, held: AllocId) {
        insert_fact(&mut self.layer.sharing.projections, (holder, key, held));
    }

    fn bind_slot_views(&mut self, local: WrapperId, views: &[AllocId]) {
        self.layer
            .sharing
            .bindings
            .retain(|&(owner, _)| owner != local);
        for &view in views {
            insert_fact(&mut self.layer.sharing.bindings, (local, view));
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
        self.resolve_local_allocs_from_cells(local, &addresses, None)
    }

    pub(in super::super) fn resolve_local_allocs_from_cells(
        &self,
        local: usize,
        addresses: &AddressSet,
        readonly_source: Option<usize>,
    ) -> ResolvedPlace {
        if let Some(entry) = self.layer.observation(local) {
            return ResolvedPlace {
                ids: entry.values.iter().map(|value| value.id).collect(),
                views: entry.views.clone(),
                layer_only: true,
                readonly: readonly_source.is_some()
                    || entry.readonly
                    || entry.values.iter().any(|value| !value.writable),
                readonly_source,
                complete: true,
            };
        }
        let mut result = ResolvedPlace {
            ids: Vec::new(),
            views: Vec::new(),
            layer_only: false,
            readonly: readonly_source.is_some(),
            readonly_source,
            complete: addresses.coverage == Coverage::Complete,
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
                        complete: true,
                    }
                }
                CellId::Projected(holder, key) => {
                    let mut projected = ResolvedPlace {
                        ids: vec![holder],
                        views: Vec::new(),
                        layer_only: true,
                        readonly: false,
                        readonly_source: None,
                        complete: true,
                    };
                    self.step_resolved(&mut projected, key, &[]);
                    projected
                }
            };
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
            result.complete &= value.complete;
        }
        if addresses.coverage == Coverage::Partial && self.locals[local].init != Init::No {
            let mut table = self.layer.table.borrow_mut();
            result.ids.extend(
                self.locals[local]
                    .value_roots
                    .iter_ones()
                    .map(|root| table.root(root)),
            );
        }
        result.ids.sort_unstable();
        result.ids.dedup();
        result.views.sort_unstable();
        result.views.dedup();
        result
    }

    pub(in super::super) fn resolve_alloc_path(
        &self,
        local_index: usize,
        local_mutable: &[bool],
        keys: impl IntoIterator<Item = FieldKey>,
    ) -> ResolvedPlace {
        let mut resolved = self.resolve_local_allocs(local_index);
        for key in keys {
            self.step_resolved(&mut resolved, key, local_mutable);
        }
        resolved
    }

    /// Allocations in a field, without following permissions or correlated views.
    pub(in super::super) fn alloc_field_ids(
        &self,
        holders: &[AllocId],
        key: FieldKey,
    ) -> Vec<AllocId> {
        let mut table = self.layer.table.borrow_mut();
        let mut ids = Vec::with_capacity(holders.len());
        for &holder in holders {
            self.layer.project_alloc(&mut table, holder, key, &mut ids);
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Advance `resolved` through one field.
    pub(in super::super) fn step_resolved(
        &self,
        resolved: &mut ResolvedPlace,
        key: FieldKey,
        local_mutable: &[bool],
    ) {
        let mut table = self.layer.table.borrow_mut();
        let views = if self.layer.sharing.is_empty() {
            Vec::new()
        } else {
            self.layer
                .project_views(&mut table, &resolved.ids, &resolved.views, key)
        };
        let mut next = Vec::with_capacity(resolved.ids.len());
        for holder in resolved.ids.drain(..) {
            let group = self.layer.group(holder, key);
            if group.is_empty() {
                next.push(table.child(holder, key));
                continue;
            }
            resolved.layer_only = true;
            resolved.complete &= group.iter().all(|edge| edge.held != AllocId::EMPTY);
            for edge in group {
                if edge.held == AllocId::EMPTY {
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
        self.layer
            .detached
            .retain(|(owner, _)| *owner != local_index as u32);
        self.layer
            .bound
            .extend(ids.iter().map(|&id| (slot, id, readonly)));
        if ids.is_empty() {
            self.layer.bound.push((slot, AllocId::EMPTY, readonly));
        }
        normalize_bindings(&mut self.layer.bound);
    }

    pub(in super::super) fn has_layer_binding(&self, local: usize) -> bool {
        let wrapper = WrapperId::current(local);
        self.binding_row(wrapper).is_some()
            || !self.layer.bound_of(wrapper).is_empty()
            || self.layer.sharing.bindings(wrapper).next().is_some()
    }

    fn clear_slot_payload(&mut self, slot: WrapperId) {
        self.layer.bound.retain(|(owner, _, _)| *owner != slot);
        self.layer
            .sharing
            .bindings
            .retain(|&(owner, _)| owner != slot);
    }

    pub(in super::super) fn stability_holders(
        &self,
        local: usize,
    ) -> impl Iterator<Item = AllocId> + '_ {
        let local = local as u32;
        let start = self
            .layer
            .stability
            .partition_point(|&(owner, _)| owner < local);
        self.layer.stability[start..]
            .iter()
            .take_while(move |&&(owner, _)| owner == local)
            .map(|&(_, holder)| holder)
    }

    pub(in super::super) fn add_stability_loans(
        &mut self,
        local: usize,
        holders: impl IntoIterator<Item = AllocId>,
    ) {
        for holder in holders {
            insert_fact(&mut self.layer.stability, (local as u32, holder));
        }
    }

    pub(in super::super) fn stability_overlaps(
        &self,
        local: usize,
        targets: &[MutationTarget],
    ) -> bool {
        if self.layer.stability.is_empty() || targets.is_empty() {
            return false;
        }
        let table = self.layer.table.borrow();
        self.stability_holders(local).any(|holder| {
            targets.iter().any(|target| {
                target.id == holder || (target.deep && self.layer.covers(&table, target.id, holder))
            })
        })
    }

    pub(in super::super) fn detached_holders(
        &self,
        local: usize,
    ) -> impl Iterator<Item = AllocId> + '_ {
        self.layer
            .detached
            .iter()
            .filter_map(move |&(owner, holder)| (owner == local as u32).then_some(holder))
    }

    pub(in super::super) fn bind_detached_holders(
        &mut self,
        local: usize,
        holders: impl IntoIterator<Item = AllocId>,
    ) {
        self.layer
            .detached
            .retain(|(owner, _)| *owner != local as u32);
        self.layer
            .detached
            .extend(holders.into_iter().map(|holder| (local as u32, holder)));
        self.layer.detached.sort_unstable();
    }

    pub(in super::super) fn invalidate_detachments(&mut self) {
        self.layer.detached.clear();
    }

    pub(in super::super) fn detached_from(&self, local: usize, holders: &[AllocId]) -> bool {
        !holders.is_empty()
            && holders.iter().all(|&holder| {
                self.layer
                    .detached
                    .binary_search(&(local as u32, holder))
                    .is_ok()
            })
    }

    fn graphs_overlap(&self, left: &[AllocId], right: &[AllocId]) -> bool {
        let mut targets = right.to_vec();
        targets.extend(self.alloc_closure(right, false));
        let targets = targets
            .into_iter()
            .map(|id| MutationTarget {
                id,
                layer_reached: true,
                deep: true,
            })
            .collect::<Vec<_>>();
        self.alloc_reaches(left, true, &targets)
    }

    fn proves_detached(&self, (local, holder): (u32, AllocId)) -> bool {
        self.layer.detached.binary_search(&(local, holder)).is_ok()
            || !self.graphs_overlap(&[holder], &self.local_alloc_ids(local as usize).0)
    }

    /// Install a new occurrence without changing a wrapper still reached by another name.
    pub(in super::super) fn rebind_local(
        &mut self,
        local_index: usize,
        new_state: LocalState,
        order: OrderKey,
    ) -> AllocationRetirement {
        let retirement = if self.layer.is_empty()
            || (self.locals[local_index].init == Init::No
                && !self.alloc_root_is_interned(local_index))
        {
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
        // Reads or capture can intern the old root. Every descendant requires that root too.
        let retirement = if pristine_definition == Some(local_index)
            && !self.alloc_root_is_interned(local_index)
            && self
                .binding_row(WrapperId::current(local_index).retired(|_| true))
                .is_none()
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

    /// Whether nothing in the layer can refer to this binding's old allocation or cells.
    ///
    /// The caller must independently rule out coarse observers. An absent root key excludes
    /// every projected descendant too: interning a child requires its parent to exist first.
    pub(in super::super) fn has_pristine_alloc_binding(&self, local_index: usize) -> bool {
        let wrapper = WrapperId::current(local_index);
        self.locals[local_index].init == Init::No
            && !self.alloc_root_is_interned(local_index)
            && self.binding_row(wrapper).is_none()
            && self.layer.bound_of(wrapper).is_empty()
    }

    fn alloc_root_is_interned(&self, local_index: usize) -> bool {
        self.layer
            .table
            .borrow()
            .index
            .contains_key(&AllocKey::Base(AllocBase::Root(local_index as u32)))
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
            });
        row.wrapper = prior;
        if self.locals[current.local].init == Init::Maybe {
            row.coverage = Coverage::Partial;
        }
        if row.has_slot() {
            let explicit_prior = !self.layer.bound_of(prior).is_empty();
            let prior_has_views = self.layer.sharing.bindings(prior).next().is_some();
            let mut views: Vec<_> = self.layer.sharing.bindings(current).collect();
            let payload = self.layer.bound_of(current);
            let mut values: Vec<_> = if !payload.is_empty() {
                payload
                    .iter()
                    .map(|&(_, id, readonly)| (prior, id, readonly))
                    .collect()
            } else if explicit_prior
                || !self.slot_roots(current).is_empty()
                || self.alloc_root_is_interned(current.local)
            {
                self.slot_values(current)
                    .map(|value| (prior, value.id, !value.writable))
                    .collect()
            } else {
                Vec::new()
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
                self.layer.bound.extend(values);
                normalize_bindings(&mut self.layer.bound);
            }
            for view in views {
                insert_fact(&mut self.layer.sharing.bindings, (prior, view));
            }
            row.slot_roots = Some(RootSet::empty(self.locals.len()));
        }
        if let Some(old) = self.binding_row(prior) {
            row.union_alts(old.alts.as_slice());
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

    fn age_wrappers(&mut self, retires: impl Fn(usize) -> bool) {
        for row in &mut self.layer.wrappers {
            row.alts.remap(|alt| {
                let target = match alt.target {
                    BindingTarget::Link(source) => BindingTarget::Link(source.retired(&retires)),
                    target => target,
                };
                let origin = alt.origin.map(|origin| Origin {
                    wrapper: origin.wrapper.retired(&retires),
                    ..origin
                });
                BindingAlt {
                    target,
                    origin,
                    ..alt
                }
            });
        }
    }

    fn freeze_slot_observers(&mut self, local: usize, keep_name: impl Fn(usize) -> bool) {
        // Root-only snapshots need an override before the old allocation is renamed.
        // Whole links are not snapshots and still load the cell.
        let first = self.layer.bound.len();
        for observer in 0..self.locals.len() {
            let slot = WrapperId::current(observer);
            if observer == local
                || !keep_name(observer)
                || !self.binding_mode(observer).contains(LocalMode::SLOT)
                || !self.slot_roots(slot).contains(local)
                || self.layer.bound[..first]
                    .binary_search_by_key(&slot, |&(owner, _, _)| owner)
                    .is_ok()
            {
                continue;
            }
            let roots = self.slot_roots(slot).clone();
            let mut table = self.layer.table.borrow_mut();
            self.layer.bound.extend(
                roots
                    .iter_ones()
                    .map(|root| (slot, table.root(root), false)),
            );
        }
        normalize_bindings(&mut self.layer.bound);
    }

    /// Replace the allocation contents without retiring the enclosing storage cell.
    pub(in super::super) fn retire_slot_value(
        &mut self,
        local: usize,
        order: OrderKey,
    ) -> AllocationRetirement {
        self.freeze_slot_observers(local, |_| true);
        self.layer.rename_base(
            AllocBase::Root(local as u32),
            AllocBase::Old(local as u32, order),
            false,
        );
        self.invalidate_observations(
            &[CellId::Slot(WrapperId::current(local))],
            Coverage::Complete,
        );
        self.clear_slot_payload(WrapperId::current(local));
        self.layer
            .detached
            .retain(|&(owner, _)| owner != local as u32);
        self.layer
            .stability
            .retain(|&(owner, _)| owner != local as u32);
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
        if self.locals[local].init != Init::No
            && self.layer.wrappers.iter().any(|row| {
                row.wrapper != current
                    && row
                        .alts
                        .as_slice()
                        .iter()
                        .any(|alt| alt.target == BindingTarget::Link(current))
            })
        {
            self.materialize_prior(current);
        }
        self.retain_observations_with_unchanged_route(|wrapper| wrapper == current);
        let retirement = self.retire_slot_value(local, order);
        self.age_wrappers(|index| index == local);
        self.remove_current_row(local);
        for state in &mut self.locals {
            state.derived_from_roots.remove(local);
        }
        AllocationRetirement {
            wrapper: true,
            ..retirement
        }
    }

    fn retain_observations_with_unchanged_route(&mut self, retires: impl Fn(WrapperId) -> bool) {
        if self.layer.observations.is_empty() {
            return;
        }
        let mut observations = std::mem::take(&mut self.layer.observations);
        observations.retain(|entry| {
            !self
                .reachable_from(vec![WrapperId::current(entry.local)])
                .into_iter()
                .any(&retires)
        });
        self.layer.observations = observations;
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
        for holder in holders {
            if strong {
                self.layer
                    .edges
                    .retain(|edge| !(edge.holder == *holder && edge.key == key));
            } else if self.layer.group(*holder, key).is_empty() {
                let lazy = self.layer.table.borrow_mut().child(*holder, key);
                self.layer.add_edge(Edge {
                    holder: *holder,
                    key,
                    held: lazy,
                    writable: true,
                    single: false,
                });
            } else {
                // A store into a known-empty group makes it non-empty.
                self.layer.edges.retain(|edge| {
                    !(edge.holder == *holder && edge.key == key && edge.held == AllocId::EMPTY)
                });
            }
            for value in values {
                let single = distinct_occurrence && self.alloc_singular(value.id);
                self.layer.insert_edge(
                    Edge {
                        holder: *holder,
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
                    holder: *holder,
                    key,
                    held: AllocId::EMPTY,
                    writable: true,
                    single: true,
                });
            }
        }
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
        let mut affected: Vec<_> = sources
            .members
            .iter()
            .filter_map(|&(view, id)| holders.contains(&id).then_some(view))
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
                    .projections
                    .retain(|&(holder, field, _)| holder != view || field != key);
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
        let tracked = self.layer.sharing.projections(holder, key).next().is_some();
        if !tracked && !views.is_empty() {
            let previous: Vec<_> = self
                .layer
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
            || (!self.layer.sharing.projections.is_empty() && {
                let targets: Vec<_> = self
                    .layer
                    .sharing
                    .projections
                    .iter()
                    .map(|&(id, _, _)| MutationTarget {
                        id,
                        layer_reached: true,
                        deep: false,
                    })
                    .collect();
                self.alloc_reaches(sources, true, &targets)
            });
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
        !self.layer.group(holder, key).is_empty()
    }

    /// Publish a captured payload without re-evaluating the operation's destination.
    pub(in super::super) fn publish_store(
        &mut self,
        operation: &TypedStore,
        order: OrderKey,
        retires: Option<&dyn Fn(usize) -> bool>,
    ) {
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
                        self.erase_alloc_family(receiver, *keys);
                        return;
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
            }
            TypedStore::Write(write) => self.publish_write(write, order, retires),
        }
    }

    fn publish_write(
        &mut self,
        write: &TypedWrite,
        order: OrderKey,
        retires: Option<&dyn Fn(usize) -> bool>,
    ) {
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
        let remap_cell = |cell| match cell {
            CellId::Projected(holder, key) => CellId::Projected(remap(holder), key),
            CellId::Slot(wrapper) => {
                CellId::Slot(wrapper.retired(|local| retires.is_some_and(|retire| retire(local))))
            }
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
                self.locals[local].value_roots = if roots.is_empty() {
                    // Report fresh payload origins rather than possible old value roots.
                    self.binding(local).overlap_roots()
                } else {
                    roots.clone()
                };
            }
            if definite.is_none() {
                let mut entry = Observation {
                    local,
                    values: values.into_owned(),
                    views: views.into_owned(),
                    readonly,
                };
                normalize_observation(&mut entry);
                let index = self
                    .layer
                    .observations
                    .partition_point(|other| other.local < local);
                self.layer.observations.insert(index, entry);
            }
        }
    }

    fn invalidate_observations(&mut self, cells: &[CellId], coverage: Coverage) {
        if self.layer.observations.is_empty() {
            return;
        }
        if coverage == Coverage::Partial {
            self.layer.observations.clear();
            return;
        }
        let mut observations = std::mem::take(&mut self.layer.observations);
        observations.retain(|entry| {
            let observed = self.resolve_cells(entry.local);
            observed.coverage == Coverage::Complete
                && !observed.cells().iter().any(|&left| {
                    cells.iter().any(|&right| match (left, right) {
                        (CellId::Slot(a), CellId::Slot(b)) => a == b,
                        (CellId::Projected(a, x), CellId::Projected(b, y)) => {
                            x == y && self.alloc_paths_may_share(&[a], &[], &[b], &[])
                        }
                        _ => false,
                    })
                })
        });
        self.layer.observations = observations;
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
                    self.layer.wrappers[index].slot_roots = Some(merged.clone());
                    self.locals[slot.local].value_roots.union_with(&merged);
                } else {
                    self.locals[slot.local].value_roots = merged;
                }
            }
            self.layer
                .detached
                .retain(|&(owner, _)| owner != slot.local as u32);
        }
        if exact {
            self.clear_slot_payload(slot);
            if values.is_empty() && views.is_empty() && !readonly && !explicit_slot_payload {
                // Plain scalar updates keep the implicit slot and allocate no sparse facts.
                return;
            }
        }
        if !exact && retained.is_empty() && !views.is_empty() {
            retained.extend(old.iter().map(|value| value.id));
        }
        if !exact && self.layer.bound_of(slot).is_empty() {
            self.layer.bound.extend(
                old.into_iter()
                    .map(|value| (slot, value.id, !value.writable)),
            );
        }
        if exact && values.is_empty() {
            self.layer.bound.push((slot, AllocId::EMPTY, readonly));
        }
        self.layer.bound.extend(
            values
                .iter()
                .map(|value| (slot, value.id, readonly || !value.writable)),
        );
        normalize_bindings(&mut self.layer.bound);
        if retained.is_empty() {
            if !views.is_empty() {
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

    fn erase_alloc_family(&mut self, holder: AllocId, keys: EntryFamilies) {
        self.invalidate_observations(
            &[
                CellId::Projected(holder, FieldKey::Elem),
                CellId::Projected(holder, FieldKey::MapKey),
            ],
            Coverage::Complete,
        );
        self.store_alloc_field(&[holder], FieldKey::Elem, &[], true);
        if matches!(keys, EntryFamilies::MapEntries) {
            self.store_alloc_field(&[holder], FieldKey::MapKey, &[], true);
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
    ) {
        let [holder] = holders else {
            return;
        };
        let group = self.layer.group(*holder, FieldKey::Elem);
        if let [edge] = group
            && edge.single
            && edge.held != AllocId::EMPTY
            && self.alloc_singular(*holder)
        {
            let holder = *holder;
            self.erase_alloc_family(
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
        }
    }

    /// Whether any allocation reached from `from` is one of `targets`.
    ///
    /// WHAT: walks lazy descendants and explicit edges. A target marked `layer_reached` matches
    ///       any reach; another target matches only when `from_layer_only` or the walk crossed an
    ///       explicit edge, because root provenance already decides the plain case.
    pub(in super::super) fn alloc_reaches(
        &self,
        from: &[AllocId],
        from_layer_only: bool,
        targets: &[MutationTarget],
    ) -> bool {
        if targets.is_empty() || from.is_empty() {
            return false;
        }
        let table = self.layer.table.borrow();
        let mut visited = FxHashSet::default();
        let mut stack: Vec<(AllocId, bool)> =
            from.iter().map(|id| (*id, from_layer_only)).collect();
        while let Some((current, crossed)) = stack.pop() {
            if !visited.insert((current, crossed)) {
                continue;
            }
            if targets.iter().any(|target| {
                (target.layer_reached || crossed)
                    && (self.layer.covers(&table, current, target.id)
                        || (target.deep && self.layer.covers(&table, target.id, current)))
            }) {
                return true;
            }
            for edge in &self.layer.edges {
                if edge.held != AllocId::EMPTY && self.layer.covers(&table, current, edge.holder) {
                    stack.push((edge.held, true));
                }
            }
        }
        false
    }

    /// Allocations reachable from `ids` through writable explicit edges, excluding `ids`.
    pub(in super::super) fn alloc_writable_closure(&self, ids: &[AllocId]) -> Vec<AllocId> {
        self.alloc_closure(ids, true)
    }

    pub(in super::super) fn alloc_readable_closure(&self, ids: &[AllocId]) -> Vec<AllocId> {
        self.alloc_closure(ids, false)
    }

    fn alloc_closure(&self, ids: &[AllocId], writable_only: bool) -> Vec<AllocId> {
        let table = self.layer.table.borrow();
        let mut seen: FxHashSet<AllocId> = ids.iter().copied().collect();
        let mut stack = ids.to_vec();
        let mut closure = Vec::new();
        while let Some(current) = stack.pop() {
            for edge in &self.layer.edges {
                if (!writable_only || edge.writable)
                    && edge.held != AllocId::EMPTY
                    && self.layer.covers(&table, current, edge.holder)
                    && seen.insert(edge.held)
                {
                    closure.push(edge.held);
                    stack.push(edge.held);
                }
            }
        }
        closure
    }

    /// Join sparse facts from the pre-join inputs, then publish the merged local facts.
    pub(in super::super) fn join_in_place(&mut self, other: &Self) -> bool {
        let mut changed = self.join_layer(other);
        let wrappers = self.merged_wrappers(other);
        changed |= self.join_local_facts(other);
        if let Some(wrappers) = wrappers {
            self.layer.wrappers = wrappers;
            changed = true;
        }
        changed
    }

    fn join_layer(&mut self, other: &Self) -> bool {
        if self.layer.is_empty() && other.layer.is_empty() {
            return false;
        }
        let mut changed = self.join_sharing(other);

        // Mix the receiver and result only after checking detachment on each incoming path.
        let mut detached = self.layer.detached.clone();
        detached.extend(other.layer.detached.iter().copied());
        detached.sort_unstable();
        detached.dedup();
        detached.retain(|pair| self.proves_detached(*pair) && other.proves_detached(*pair));
        changed |= detached != self.layer.detached;
        self.layer.detached = detached;

        // An overridden slot holds its ordinary contents on the opposite incoming path.
        // Equal groups need neither a fallback nor a merge.
        if self.layer.bound != other.layer.bound {
            let mut bound = self.layer.bound.clone();
            for (source, opposite) in [(&*self, other), (other, &*self)] {
                let mut previous = None;
                for &(slot, _, _) in &source.layer.bound {
                    if previous.replace(slot) != Some(slot)
                        && opposite.owns_slot(slot)
                        && opposite.layer.bound_of(slot).is_empty()
                    {
                        bound.extend(
                            opposite
                                .slot_values(slot)
                                .map(|value| (slot, value.id, !value.writable)),
                        );
                    }
                }
            }
            bound.extend(other.layer.bound.iter().copied());
            normalize_bindings(&mut bound);
            changed |= bound != self.layer.bound;
            self.layer.bound = bound;
        }

        // A group explicit on one side holds the lazy child on the other.
        let mut merged = Vec::with_capacity(self.layer.edges.len() + other.layer.edges.len());
        let mut table = self.layer.table.borrow_mut();
        for (source, opposite) in [(&self.layer, &other.layer), (&other.layer, &self.layer)] {
            for edge in &source.edges {
                merged.push(*edge);
                if opposite.group(edge.holder, edge.key).is_empty() {
                    merged.push(Edge {
                        holder: edge.holder,
                        key: edge.key,
                        held: table.child(edge.holder, edge.key),
                        writable: true,
                        single: false,
                    });
                }
            }
        }
        drop(table);
        merged.sort_by_key(Edge::sort_key);
        merged.dedup_by(|later, earlier| {
            if later.sort_key() != earlier.sort_key() {
                return false;
            }
            earlier.writable &= later.writable;
            earlier.single &= later.single;
            true
        });
        changed |= merged != self.layer.edges;
        self.layer.edges = merged;

        let previous_len = self.layer.observations.len();
        self.layer.observations.retain_mut(|entry| {
            let Some(incoming) = other.layer.observation(entry.local) else {
                return false;
            };
            if entry != incoming {
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
                entry.views.extend_from_slice(&incoming.views);
                entry.values.extend_from_slice(&incoming.values);
                entry.readonly |= incoming.readonly;
                normalize_observation(entry);
                changed |= *entry != old;
            }
            true
        });
        changed |= self.layer.observations.len() != previous_len;
        let stability_len = self.layer.stability.len();
        for &loan in &other.layer.stability {
            insert_fact(&mut self.layer.stability, loan);
        }
        changed |= stability_len != self.layer.stability.len();
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
                row.union_alts(b.alts.as_slice());
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
            sharing.members.extend_from_slice(&source.members);
            sharing.bindings.extend_from_slice(&source.bindings);
            sharing.projections.extend_from_slice(&source.projections);
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
            let mut last_group = None;
            for &(holder, key, _) in &source.projections {
                if last_group.replace((holder, key)) == Some((holder, key))
                    || opposite.projections(holder, key).next().is_some()
                    || (matches!(table.base(holder), AllocBase::CallView(..))
                        && opposite.members(holder).next().is_none())
                {
                    continue;
                }
                let mut ids = Vec::new();
                opposite_state
                    .layer
                    .project_members(&mut table, holder, key, &mut ids);
                sharing
                    .projections
                    .extend(ids.into_iter().map(|id| (holder, key, id)));
            }
            for &(a, b, shares) in &source.pairs {
                let could_exist = |view| {
                    !matches!(table.base(view), AllocBase::CallView(..))
                        || opposite.members(view).next().is_some()
                };
                let possible = shares
                    || (could_exist(a)
                        && could_exist(b)
                        && opposite_state.layer.views_may_share(a, b));
                sharing.union_pair(a, b, possible);
            }
        }
        sharing.members.sort_unstable();
        sharing.members.dedup();
        sharing.bindings.sort_unstable();
        sharing.bindings.dedup();
        sharing.projections.sort_unstable();
        sharing.projections.dedup();
        let changed = &sharing != mine;
        self.layer.sharing = sharing;
        changed
    }

    /// Keep departing bindings' allocations named and drop facts no visible binding reaches.
    ///
    /// WHY: runs before the locals are narrowed. A visible binding whose roots name a departing
    ///      binding keeps that allocation as an override, and every id rooted at the departing
    ///      binding moves to a base that outlives the name.
    pub(super) fn narrow_layer(&mut self, visible: &RootSet) {
        if self.layer.is_empty() {
            return;
        }
        let reached = self.reachable_wrappers(|local| visible.contains(local));
        self.retain_observations_with_unchanged_route(|wrapper| {
            wrapper.age == WrapperAge::Current && !visible.contains(wrapper.local)
        });
        // First capture all departing slot payloads in the pre-edge allocation coordinates.
        for local in 0..self.locals.len() {
            let wrapper = WrapperId::current(local);
            if !visible.contains(local)
                && self.locals[local].init != Init::No
                && reached.contains(&wrapper)
            {
                self.materialize_prior(wrapper);
            }
        }
        for local in 0..self.locals.len() {
            if visible.contains(local) {
                continue;
            }
            if self.alloc_root_is_interned(local) {
                self.freeze_slot_observers(local, |observer| visible.contains(observer));
                self.layer.rename_base(
                    AllocBase::Root(local as u32),
                    AllocBase::Departed(local as u32),
                    false,
                );
            }
        }
        self.age_wrappers(|local| !visible.contains(local));
        self.prune_wrappers(|local| visible.contains(local));
        for row in &mut self.layer.wrappers {
            row.route_roots.intersect_with(visible);
            if let Some(roots) = &mut row.slot_roots {
                roots.intersect_with(visible);
            }
        }
        self.layer
            .observations
            .retain(|entry| visible.contains(entry.local));
        self.layer
            .stability
            .retain(|&(owner, _)| visible.contains(owner as usize));
        self.layer
            .detached
            .retain(|&(owner, _)| visible.contains(owner as usize));
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
        self.layer.wrappers.retain(|row| keep(row.wrapper));
        self.layer.bound.retain(|&(slot, _, _)| keep(slot));
        self.layer.sharing.bindings.retain(|&(slot, _)| keep(slot));
    }

    pub(in super::super) fn prune_retired_wrappers(&mut self) {
        if self
            .layer
            .wrappers
            .iter()
            .any(|row| row.wrapper.age == WrapperAge::Prior)
        {
            self.prune_wrappers(|_| true);
        }
    }

    /// Drop edges whose holder no visible binding reaches.
    fn collect_unreachable_allocations(&mut self, visible: &super::RootSet) {
        if self.layer.edges.is_empty() && self.layer.sharing.is_empty() {
            return;
        }
        let mut live: Vec<AllocId> = Vec::new();
        for local in visible.iter_ones() {
            if self.locals[local].init == Init::No {
                continue;
            }
            live.extend(self.local_alloc_ids(local).0);
        }
        live.extend(self.layer.projected_holders());
        live.extend(
            self.layer
                .bound
                .iter()
                .filter_map(|&(_, id, _)| (id != AllocId::EMPTY).then_some(id)),
        );
        live.extend(self.layer.stability.iter().map(|&(_, holder)| holder));
        let table = self.layer.table.borrow();
        let mut reached: FxHashSet<AllocId> = live.iter().copied().collect();
        let mut stack = live;
        let mut keep = vec![false; self.layer.edges.len()];
        while let Some(current) = stack.pop() {
            for (index, edge) in self.layer.edges.iter().enumerate() {
                if keep[index] || !ancestor_or_self(&table, current, edge.holder) {
                    continue;
                }
                keep[index] = true;
                if edge.held != AllocId::EMPTY && reached.insert(edge.held) {
                    stack.push(edge.held);
                }
            }
        }
        if !self.layer.sharing.is_empty() {
            let sharing = &mut self.layer.sharing;
            let mut live_views = FxHashSet::default();
            let mut visit: Vec<AllocId> = sharing.bindings.iter().map(|&(_, view)| view).collect();
            for &(holder, _, _) in &sharing.projections {
                if reached
                    .iter()
                    .any(|&id| ancestor_or_self(&table, id, holder))
                {
                    visit.push(holder);
                }
            }
            while let Some(view) = visit.pop() {
                if !live_views.insert(view) {
                    continue;
                }
                let start = sharing
                    .projections
                    .partition_point(|&(holder, _, _)| holder < view);
                visit.extend(
                    sharing.projections[start..]
                        .iter()
                        .take_while(|&&(holder, _, _)| holder == view)
                        .map(|&(_, _, child)| child),
                );
            }
            sharing
                .members
                .retain(|&(view, _)| live_views.contains(&view));
            sharing
                .projections
                .retain(|&(holder, _, _)| live_views.contains(&holder));
            sharing
                .pairs
                .retain(|&(a, b, _)| live_views.contains(&a) && live_views.contains(&b));
        }
        drop(table);
        let mut index = 0;
        self.layer.edges.retain(|_| {
            let kept = keep[index];
            index += 1;
            kept
        });
    }
}

/// Whether `ancestor` is `id` or on its field path, ignoring overrides.
///
/// WHY: collection keeps an edge whenever its holder might be reached; an override that hides
///      the path only makes the kept edge unreachable for conflicts, never wrong.
fn ancestor_or_self(table: &AllocTable, ancestor: AllocId, id: AllocId) -> bool {
    let mut current = id;
    loop {
        if current == ancestor {
            return true;
        }
        match table.parent(current) {
            Some((parent, _)) => current = parent,
            None => return false,
        }
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
fn cyclic_blocks(successors: &FxHashMap<BlockId, Vec<BlockId>>) -> FxHashSet<BlockId> {
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
        assert_eq!(state.layer.bound.capacity(), 0);
        assert!(state.layer.table.borrow().infos.is_empty());
    }
}
