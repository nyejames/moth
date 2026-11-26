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

use super::{BorrowState, LocalState, OrderKey};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::ids::BlockId;
use rustc_hash::{FxHashMap, FxHashSet};
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
    Call(OrderKey, u32),
    /// Any unproven allocation of the given semantic type returned by one call.
    OpaqueCall(OrderKey, u32),
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

    fn copied_node(&mut self, graph: AllocId, source: AllocId, singular: bool) -> AllocId {
        self.intern(AllocKey::Copied(graph, source), self.base(graph), singular)
    }

    fn parent(&self, id: AllocId) -> Option<(AllocId, FieldKey)> {
        match self.infos[id.0 as usize].key {
            AllocKey::Child(parent, key) => Some((parent, key)),
            AllocKey::Base(_) | AllocKey::Copied(_, _) => None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AliasCell {
    local: u32,
    holder: AllocId,
    key: FieldKey,
    /// Every incoming path writes through this cell.
    definite: bool,
}

impl AliasCell {
    fn sort_key(self) -> (u32, AllocId, FieldKey) {
        (self.local, self.holder, self.key)
    }
}

/// Explicit holder edges, allocation overrides for bindings, and projected mutable alias cells.
#[derive(Debug, Clone)]
pub(in super::super) struct AllocationLayer {
    table: Rc<RefCell<AllocTable>>,
    edges: Vec<Edge>,
    /// Allocations and retained permission when the binding's roots cannot name them.
    bound: Vec<(u32, AllocId, bool)>,
    /// The (holder, field) cells a mutable alias of a projected place writes through.
    cells: Vec<AliasCell>,
    /// (Result binding, former holder) pairs whose reachable graphs are disjoint on every path.
    detached: Vec<(u32, AllocId)>,
}

impl PartialEq for AllocationLayer {
    fn eq(&self, other: &Self) -> bool {
        self.edges == other.edges
            && self.bound == other.bound
            && self.cells == other.cells
            && self.detached == other.detached
    }
}

impl Eq for AllocationLayer {}

impl AllocationLayer {
    pub(super) fn new() -> Self {
        Self {
            table: Rc::new(RefCell::new(AllocTable::default())),
            edges: Vec::new(),
            bound: Vec::new(),
            cells: Vec::new(),
            detached: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.edges.is_empty()
            && self.bound.is_empty()
            && self.cells.is_empty()
            && self.detached.is_empty()
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

    fn bound_of(&self, local: usize) -> &[(u32, AllocId, bool)] {
        let local = local as u32;
        let start = self.bound.partition_point(|(owner, _, _)| *owner < local);
        let end = start
            + self.bound[start..]
                .iter()
                .take_while(|(owner, _, _)| *owner == local)
                .count();
        &self.bound[start..end]
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
        }) || self.bound.iter().any(|&(_, id, _)| table.base(id) == base)
            || self
                .cells
                .iter()
                .any(|cell| table.base(cell.holder) == base);
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
            || self.cells.iter().any(|cell| mentions(cell.holder));
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
        for cell in &mut self.cells {
            cell.holder = rename(cell.holder, &mut table);
        }
        self.cells.sort_unstable();
        self.cells.dedup_by_key(|cell| cell.sort_key());
    }
}

fn normalize_bindings(bindings: &mut Vec<(u32, AllocId, bool)>) {
    bindings.sort_unstable();
    bindings.dedup_by(|later, earlier| {
        if (later.0, later.1) != (earlier.0, earlier.1) {
            return false;
        }
        earlier.2 |= later.2;
        true
    });
}

/// Allocations a place reaches, with how the layer reached them.
#[derive(Debug, Clone)]
pub(in super::super) struct ResolvedPlace {
    pub ids: Vec<AllocId>,
    /// Whether the ids depend on layer facts that root provenance does not carry: a binding
    /// override or an explicit holder edge.
    pub layer_only: bool,
    /// Whether the path passes through a value stored from an immutable binding.
    pub readonly: bool,
    /// The binding that stored value came from, when the layer still names it.
    pub readonly_source: Option<usize>,
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
}

impl AllocationRetirement {
    pub(in super::super) fn remap_id(self, state: &BorrowState, id: &mut AllocId) {
        *id = state.remap_retired_alloc_id(*id, self.order, |local| local == self.local);
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

impl BorrowState {
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
        node: u32,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> AllocId {
        self.stable_site_id(AllocBase::Call(order, node), block, successors)
    }

    pub(in super::super) fn opaque_call_id(&self, order: OrderKey, ty: u32) -> AllocId {
        self.layer
            .table
            .borrow_mut()
            .base_id(AllocBase::OpaqueCall(order, ty), false)
    }

    fn stable_site_id(
        &self,
        base: AllocBase,
        block: BlockId,
        successors: &FxHashMap<BlockId, Vec<BlockId>>,
    ) -> AllocId {
        let mut table = self.layer.table.borrow_mut();
        let cyclic = table
            .cyclic_blocks
            .get_or_insert_with(|| cyclic_blocks(successors));
        let singular = !cyclic.contains(&block);
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
        let bound = self.layer.bound_of(local_index);
        if !bound.is_empty() {
            return (bound.iter().map(|(_, id, _)| *id).collect(), true);
        }
        let mut table = self.layer.table.borrow_mut();
        let ids = self
            .effective_roots(local_index)
            .iter_ones()
            .map(|root| table.root(root))
            .collect();
        (ids, false)
    }

    pub(in super::super) fn alloc_binding_readonly(&self, local: usize) -> bool {
        self.layer
            .bound_of(local)
            .iter()
            .any(|(_, _, readonly)| *readonly)
    }

    /// Allocations reached by the local itself, before following any projections.
    pub(in super::super) fn resolve_local_allocs(&self, local_index: usize) -> ResolvedPlace {
        let (ids, layer_only) = self.local_alloc_ids(local_index);
        ResolvedPlace {
            ids,
            layer_only,
            readonly: self.alloc_binding_readonly(local_index),
            readonly_source: None,
        }
    }

    #[cfg(test)]
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

    /// Advance `resolved` through one field.
    pub(in super::super) fn step_resolved(
        &self,
        resolved: &mut ResolvedPlace,
        key: FieldKey,
        local_mutable: &[bool],
    ) {
        let mut table = self.layer.table.borrow_mut();
        let mut next = Vec::with_capacity(resolved.ids.len());
        for holder in resolved.ids.drain(..) {
            let group = self.layer.group(holder, key);
            if group.is_empty() {
                next.push(table.child(holder, key));
                continue;
            }
            resolved.layer_only = true;
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
    }

    /// Record that `local_index` holds `ids`, replacing any earlier override.
    pub(in super::super) fn bind_alloc_ids(
        &mut self,
        local_index: usize,
        ids: &[AllocId],
        readonly: bool,
    ) {
        self.clear_alloc_binding(local_index);
        self.layer
            .detached
            .retain(|(owner, _)| *owner != local_index as u32);
        let local = local_index as u32;
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let start = self
            .layer
            .bound
            .partition_point(|(owner, _, _)| *owner < local);
        self.layer.bound.splice(
            start..start,
            ids.into_iter().map(|id| (local, id, readonly)),
        );
    }

    /// Record the cell a mutable alias of a projected place writes through.
    pub(in super::super) fn bind_alias_cells(
        &mut self,
        local_index: usize,
        cells: &[(AllocId, FieldKey)],
        strong: bool,
        complete: bool,
    ) {
        let local = local_index as u32;
        if strong {
            self.layer.cells.retain(|cell| cell.local != local);
        } else {
            for cell in self
                .layer
                .cells
                .iter_mut()
                .filter(|cell| cell.local == local)
            {
                cell.definite = false;
            }
        }
        let definite = strong && complete && cells.len() == 1;
        self.layer
            .cells
            .extend(cells.iter().map(|&(holder, key)| AliasCell {
                local,
                holder,
                key,
                definite,
            }));
        self.layer.cells.sort_unstable();
        self.layer.cells.dedup();
    }

    pub(in super::super) fn alias_cells(&self, local_index: usize) -> Vec<(AllocId, FieldKey)> {
        let local = local_index as u32;
        let cells = &self.layer.cells;
        let start = cells.partition_point(|cell| cell.local < local);
        cells[start..]
            .iter()
            .take_while(|cell| cell.local == local)
            .map(|cell| (cell.holder, cell.key))
            .collect()
    }

    pub(in super::super) fn alias_cell_is_definite(
        &self,
        local: usize,
        cells: &[(AllocId, FieldKey)],
    ) -> bool {
        let [(holder, key)] = cells else {
            return false;
        };
        self.layer
            .cells
            .binary_search(&AliasCell {
                local: local as u32,
                holder: *holder,
                key: *key,
                definite: true,
            })
            .is_ok()
    }

    pub(in super::super) fn has_layer_binding(&self, local: usize) -> bool {
        !self.layer.bound_of(local).is_empty()
    }

    pub(in super::super) fn exact_projected_cell(
        &self,
        local: usize,
        cells: &[(AllocId, FieldKey)],
    ) -> bool {
        self.locals[local].is_alias_only()
            && self.locals[local].direct_alias_roots.is_empty()
            && self.alias_cell_is_definite(local, cells)
            && matches!(cells, [(holder, FieldKey::Field(_))] if self.alloc_singular(*holder))
    }

    fn clear_alloc_binding(&mut self, local_index: usize) {
        let local = local_index as u32;
        self.layer.bound.retain(|(owner, _, _)| *owner != local);
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

    /// Rebind `local_index` to `new_state` at `order`.
    ///
    /// WHAT: the binding's previous allocation keeps its edges under a renamed base, and the
    ///       binding's overrides and alias cells are cleared.
    /// WHY: whatever still holds the old allocation must keep observing it, while the binding
    ///      itself now names a different allocation.
    pub(in super::super) fn rebind_local(
        &mut self,
        local_index: usize,
        new_state: LocalState,
        order: OrderKey,
    ) -> AllocationRetirement {
        let retirement = if self.layer.is_empty() {
            AllocationRetirement {
                local: local_index,
                order,
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
        let local = local_index as u32;
        self.locals[local_index].mode.is_definitely_uninit()
            && !self.alloc_root_is_interned(local_index)
            && self
                .layer
                .bound
                .binary_search_by_key(&local, |&(owner, _, _)| owner)
                .is_err()
            && self
                .layer
                .cells
                .binary_search_by_key(&local, |cell| cell.local)
                .is_err()
            && self
                .layer
                .detached
                .binary_search_by_key(&local, |&(owner, _)| owner)
                .is_err()
    }

    fn alloc_root_is_interned(&self, local_index: usize) -> bool {
        self.layer
            .table
            .borrow()
            .index
            .contains_key(&AllocKey::Base(AllocBase::Root(local_index as u32)))
    }

    /// Rename the allocation `local_index` owns and clear its overrides without changing its
    /// root state.
    pub(in super::super) fn retire_binding_allocation(
        &mut self,
        local_index: usize,
        order: OrderKey,
    ) -> AllocationRetirement {
        let local = local_index as u32;
        // These observers have no override for rename_base to update. Capture their complete
        // current provenance first; direct cell aliases instead follow the binding cell.
        let binding_count = self.layer.bound.len();
        {
            let mut table = self.layer.table.borrow_mut();
            for (observer, state) in self.locals.iter().enumerate() {
                // A different binding can see this root only through its value roots.
                if observer == local_index
                    || state.mode.is_definitely_uninit()
                    || !state.value_roots.contains(local_index)
                    || state.direct_alias_roots.contains(local_index)
                    || self.layer.bound[..binding_count]
                        .binary_search_by_key(&(observer as u32), |&(owner, _, _)| owner)
                        .is_ok()
                {
                    continue;
                }
                // Membership proved the value roots are nonempty. Only a slot/alias join
                // includes the observer's own root as well.
                let includes_self = state.mode.contains(LocalMode::SLOT.union(LocalMode::ALIAS));
                let roots = includes_self
                    .then_some(observer)
                    .into_iter()
                    .chain(state.value_roots.iter_ones());
                self.layer
                    .bound
                    .extend(roots.map(|root| (observer as u32, table.root(root), false)));
            }
        }
        self.layer
            .rename_base(AllocBase::Root(local), AllocBase::Old(local, order), false);
        self.clear_alloc_binding(local_index);
        self.layer.detached.retain(|(owner, _)| *owner != local);
        self.layer.cells.retain(|cell| cell.local != local);
        AllocationRetirement {
            local: local_index,
            order,
        }
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
        if holders.is_empty() || (!strong && values.is_empty()) {
            return;
        }
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
                let single = self.alloc_singular(value.id);
                self.layer.add_edge(Edge {
                    holder: *holder,
                    key,
                    held: value.id,
                    writable: value.writable,
                    single,
                });
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

    /// Record one construction child edge into a holder that has no explicit group yet.
    pub(in super::super) fn add_alloc_edge(
        &mut self,
        holder: AllocId,
        key: FieldKey,
        value: StoredValue,
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
        self.layer.add_edge(Edge {
            holder,
            key,
            held: value.id,
            writable: value.writable,
            single,
        });
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
        sources: &[AllocId],
        site: AllocId,
    ) -> Vec<AllocId> {
        if sources.is_empty() || self.layer.edges.is_empty() {
            return Vec::new();
        }
        let mut table = self.layer.table.borrow_mut();
        let singular = table.singular(site);
        let mut copied = Vec::new();
        let mut roots = Vec::new();
        if singular {
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

    /// Add `ids` to the allocations `local_index` may hold.
    ///
    /// WHY: a write that only some paths perform keeps what the binding held before.
    pub(in super::super) fn extend_alloc_ids(
        &mut self,
        local_index: usize,
        ids: &[AllocId],
        readonly: bool,
    ) {
        let (mut held, _) = self.local_alloc_ids(local_index);
        held.extend_from_slice(ids);
        self.bind_alloc_ids(
            local_index,
            &held,
            readonly || self.alloc_binding_readonly(local_index),
        );
    }

    /// A projected observer of this slot's old allocation does not alias the new slot value.
    ///
    /// WHY: drop the conservative cell root only when every alternative names a projection
    ///      within the old allocation. A call-result root or an ambiguous alias keeps it.
    pub(in super::super) fn release_projected_observers(
        &mut self,
        local_index: usize,
        order: OrderKey,
    ) {
        if self.layer.bound.is_empty() {
            return;
        }
        let retired = AllocBase::Old(local_index as u32, order);
        let table = self.layer.table.borrow();
        for (observer, state) in self.locals.iter_mut().enumerate() {
            if observer == local_index || state.direct_alias_roots.contains(local_index) {
                continue;
            }
            let bound = self.layer.bound_of(observer);
            if !bound.is_empty()
                && bound
                    .iter()
                    .all(|(_, id, _)| table.base(*id) == retired && table.parent(*id).is_some())
            {
                state.value_roots.remove(local_index);
                state.write_target_roots.remove(local_index);
            }
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
            self.store_alloc_field(&[holder], FieldKey::Elem, &[], true);
            if map {
                self.store_alloc_field(&[holder], FieldKey::MapKey, &[], true);
            }
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

    /// Join the layer of `other` into this state's layer before the locals join.
    pub(super) fn join_layer(&mut self, other: &Self) -> bool {
        if self.layer.is_empty() && other.layer.is_empty() {
            return false;
        }
        let mut changed = false;

        // Mix the receiver and result only after checking detachment on each incoming path.
        let mut detached = self.layer.detached.clone();
        detached.extend(other.layer.detached.iter().copied());
        detached.sort_unstable();
        detached.dedup();
        detached.retain(|pair| self.proves_detached(*pair) && other.proves_detached(*pair));
        changed |= detached != self.layer.detached;
        self.layer.detached = detached;

        // A binding overridden on one side holds its root-derived allocations on the other.
        let mut bound = self.layer.bound.clone();
        for local_index in 0..self.locals.len() {
            let mine = !self.layer.bound_of(local_index).is_empty();
            let theirs = !other.layer.bound_of(local_index).is_empty();
            if mine && !theirs {
                let (ids, _) = other.local_alloc_ids(local_index);
                bound.extend(ids.into_iter().map(|id| (local_index as u32, id, false)));
            } else if theirs && !mine {
                let (ids, _) = self.local_alloc_ids(local_index);
                bound.extend(ids.into_iter().map(|id| (local_index as u32, id, false)));
            }
        }
        bound.extend(other.layer.bound.iter().copied());
        normalize_bindings(&mut bound);
        changed |= bound != self.layer.bound;
        self.layer.bound = bound;

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
        // An empty marker means nothing only when it is the whole group.
        let mut edges = Vec::with_capacity(merged.len());
        for (index, edge) in merged.iter().enumerate() {
            let in_group = |other: &Edge| other.holder == edge.holder && other.key == edge.key;
            let group_len = merged[..index]
                .iter()
                .rev()
                .take_while(|e| in_group(e))
                .count()
                + merged[index..].iter().take_while(|e| in_group(e)).count();
            if edge.held == AllocId::EMPTY && group_len > 1 {
                continue;
            }
            edges.push(*edge);
        }
        changed |= edges != self.layer.edges;
        self.layer.edges = edges;

        let mut cells = self.layer.cells.clone();
        cells.extend(other.layer.cells.iter().copied());
        for cell in &mut cells {
            cell.definite = cell.definite
                && self.layer.cells.binary_search(cell).is_ok()
                && other.layer.cells.binary_search(cell).is_ok();
        }
        cells.sort_unstable();
        cells.dedup();
        changed |= cells != self.layer.cells;
        self.layer.cells = cells;
        changed
    }

    /// Keep departing bindings' allocations named and drop facts no visible binding reaches.
    ///
    /// WHY: runs before the locals are narrowed. A visible binding whose roots name a departing
    ///      binding keeps that allocation as an override, and every id rooted at the departing
    ///      binding moves to a base that outlives the name.
    pub(super) fn narrow_layer(&mut self, visible: &super::RootSet) {
        if self.layer.is_empty() {
            return;
        }
        let departing: Vec<usize> = (0..self.locals.len())
            .filter(|local| !visible.contains(*local))
            .collect();
        if departing.is_empty() {
            return;
        }
        for &local in &departing {
            let base = AllocBase::Root(local as u32);
            let mentioned = {
                let table = self.layer.table.borrow();
                self.layer.edges.iter().any(|edge| {
                    table.base(edge.holder) == base
                        || (edge.held != AllocId::EMPTY && table.base(edge.held) == base)
                }) || self
                    .layer
                    .bound
                    .iter()
                    .any(|(_, id, _)| table.base(*id) == base)
                    || self
                        .layer
                        .cells
                        .iter()
                        .any(|cell| table.base(cell.holder) == base)
            };
            if !mentioned {
                continue;
            }
            for observer in visible.iter_ones() {
                if self.layer.bound_of(observer).is_empty()
                    && self.effective_root_contains(observer, local)
                {
                    let (ids, _) = self.local_alloc_ids(observer);
                    self.bind_alloc_ids(observer, &ids, false);
                }
            }
            self.layer
                .rename_base(base, AllocBase::Departed(local as u32), false);
        }
        let departing_set: FxHashSet<u32> = departing.iter().map(|local| *local as u32).collect();
        self.layer
            .bound
            .retain(|(owner, _, _)| !departing_set.contains(owner));
        self.layer
            .cells
            .retain(|cell| !departing_set.contains(&cell.local));
        self.layer
            .detached
            .retain(|(owner, _)| visible.contains(*owner as usize));
        self.collect_unreachable_allocations(visible);
    }

    /// Drop edges whose holder no visible binding reaches.
    fn collect_unreachable_allocations(&mut self, visible: &super::RootSet) {
        let mut live: Vec<AllocId> = Vec::new();
        for local in visible.iter_ones() {
            if self.locals[local].mode == LocalMode::UNINIT {
                continue;
            }
            live.extend(self.local_alloc_ids(local).0);
        }
        live.extend(self.layer.cells.iter().map(|cell| cell.holder));
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
        AllocBase::Site(_) | AllocBase::Call(..) | AllocBase::OpaqueCall(..) => None,
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
