//! Conflict detection helpers for borrow-transfer access checks.
//!
//! WHAT: classifies when a requested access overlaps an active borrow state.
//! WHY: transfer logic needs one place to keep borrow-conflict rules deterministic and testable.

use super::{AccessCheckContext, BORROW_ACCESS, BorrowTransferContext, MutableAccessPolicy};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, BorrowState, CellId, Coverage, FieldKey, FunctionLayout, FutureUseKind,
    PlaceResolution, RootSet, SmallSet, WrapperAge, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{
    AccessFootprint, AccessReservation, Demand, ObservationBoundary, ObservationCheckpoint,
    PlaceUse, StatementAccessTracker, ValueObservations, matching_carriers,
    visit_observed_carriers,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{AccessKind, LocalMode};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::ops::ControlFlow::{self, Break, Continue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super::super) enum Overlap {
    No,
    Yes,
    Unknown,
}

fn cell_relation(
    state: &BorrowState,
    a: &[CellId],
    a_coverage: Coverage,
    b: &[CellId],
    b_coverage: Coverage,
) -> Overlap {
    if a.iter().any(|&x| {
        b.iter().any(|&y| {
            x == y
                || matches!((x, y), (CellId::Projected(x, k), CellId::Projected(y, l))
            if k == l && state.alloc_paths_may_share(&[x], &[], &[y], &[]))
        })
    }) {
        Overlap::Yes
    } else if a.is_empty() && a_coverage == Coverage::Complete
        || b.is_empty() && b_coverage == Coverage::Complete
    {
        Overlap::No
    } else if a_coverage == Coverage::Partial || b_coverage == Coverage::Partial {
        Overlap::Unknown
    } else {
        Overlap::No
    }
}

fn writable(footprint: &AccessFootprint) -> bool {
    footprint.usage.kind() == AccessKind::Mutable
}

fn deep(footprint: &AccessFootprint) -> bool {
    matches!(
        footprint.usage,
        PlaceUse::Read | PlaceUse::Mutate { deep: true }
    )
}

fn touches_entries(key: FieldKey) -> bool {
    matches!(key, FieldKey::Elem | FieldKey::MapKey)
}

fn projected_edit_overlaps(
    state: &BorrowState,
    edit: &AccessFootprint,
    observer: GraphFootprint<'_>,
) -> Overlap {
    if !writable(edit) || !edit.usage.touches_cell() {
        return Overlap::No;
    }
    let entries = matches!(
        observer.usage,
        PlaceUse::ReadEntries | PlaceUse::WriteEntries
    );
    let descendants = observer.deep();
    if !entries && !descendants {
        return Overlap::No;
    }
    let mut projected = false;
    for &cell in edit.address.cells() {
        let CellId::Projected(holder, key) = cell else {
            continue;
        };
        if entries && !touches_entries(key) {
            continue;
        }
        projected = true;
        if state.alloc_reaches_typed(observer.ids, holder, descendants, observer.writable()) {
            return Overlap::Yes;
        }
    }
    if projected && observer.coverage == Coverage::Partial
        || edit.address.may_coverage() == Coverage::Partial
            && (!observer.ids.is_empty() || observer.coverage == Coverage::Partial)
    {
        return Overlap::Unknown;
    }
    Overlap::No
}

#[derive(Clone, Copy)]
struct GraphFootprint<'a> {
    ids: &'a [AllocId],
    views: &'a [AllocId],
    coverage: Coverage,
    usage: PlaceUse,
}

impl<'a> GraphFootprint<'a> {
    fn of(access: &'a AccessFootprint) -> Option<Self> {
        let content = access.content.as_ref()?;
        Some(Self {
            ids: &content.ids,
            views: &content.views,
            coverage: content.coverage,
            usage: access.usage,
        })
    }

    fn share_ids(self) -> &'a [AllocId] {
        if self.views.is_empty() {
            self.ids
        } else {
            self.views
        }
    }

    fn deep(self) -> bool {
        matches!(self.usage, PlaceUse::Read | PlaceUse::Mutate { deep: true })
    }

    fn writable(self) -> bool {
        self.usage.kind() == AccessKind::Mutable
    }

    fn reaches(self, state: &BorrowState, targets: &[AllocId]) -> bool {
        self.deep()
            && self.ids.iter().any(|&id| {
                targets.iter().any(|&target| {
                    id != target && state.alloc_reaches_typed(&[id], target, true, self.writable())
                })
            })
    }
}

fn graph_relation(state: &BorrowState, a: &AccessFootprint, b: &AccessFootprint) -> Overlap {
    match (GraphFootprint::of(a), GraphFootprint::of(b)) {
        (Some(a), Some(b)) => graph_overlap(state, a, b),
        _ => Overlap::No,
    }
}

fn graph_overlap(state: &BorrowState, a: GraphFootprint<'_>, b: GraphFootprint<'_>) -> Overlap {
    if state.alloc_paths_may_share(a.ids, a.views, b.ids, b.views)
        || a.reaches(state, b.ids)
        || b.reaches(state, a.ids)
    {
        return Overlap::Yes;
    }
    if a.deep() && b.deep() && state.has_alloc_edges() {
        // Distinct roots can share a child reachable from neither root's peer.
        let descendants = if a.writable() {
            state.alloc_writable_closure(a.ids)
        } else {
            state.alloc_readable_closure(a.ids)
        };
        if descendants
            .iter()
            .any(|&child| state.alloc_reaches_typed(b.ids, child, true, b.writable()))
        {
            return Overlap::Yes;
        }
    }
    if a.coverage == Coverage::Partial && (!b.ids.is_empty() || b.coverage == Coverage::Partial)
        || b.coverage == Coverage::Partial && !a.ids.is_empty()
    {
        return Overlap::Unknown;
    }
    Overlap::No
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CellIncidence {
    Slot(WrapperId),
    Projected(FieldKey, (AllocId, AllocId)),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum OperandIncidence {
    Address(CellIncidence),
    Traversal(CellIncidence),
    AddressPossible,
    AddressPartial,
    TraversalPossible,
    TraversalPartial,
    GraphShare((AllocId, AllocId)),
    EntryReceiver(AllocId),
    EntryEdit(AllocId),
    GraphPossible,
    GraphPartial,
    ProjectedEdit,
}

impl OperandIncidence {
    fn visit_matches(self, mut visit: impl FnMut(Self) -> ControlFlow<()>) -> ControlFlow<()> {
        let mut each = |key| {
            count_operand_key();
            visit(key)
        };
        use OperandIncidence::*;
        match self {
            Address(cell) => {
                each(Address(cell))?;
                each(Traversal(cell))?;
            }
            Traversal(cell) => each(Address(cell))?,
            AddressPossible => {
                each(AddressPartial)?;
                each(TraversalPartial)?;
            }
            AddressPartial => {
                each(AddressPossible)?;
                each(TraversalPossible)?;
                each(GraphPossible)?;
            }
            TraversalPossible => each(AddressPartial)?,
            TraversalPartial => each(AddressPossible)?,
            GraphShare(_) => each(self)?,
            EntryReceiver(id) => each(EntryEdit(id))?,
            EntryEdit(id) => each(EntryReceiver(id))?,
            GraphPossible => {
                each(AddressPartial)?;
                each(GraphPartial)?;
            }
            GraphPartial => {
                each(ProjectedEdit)?;
                each(GraphPossible)?;
            }
            ProjectedEdit => each(GraphPartial)?,
        }
        Continue(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum OperandGroup {
    Cell(FieldKey, AllocId, bool),
    Share(AllocId),
    Reach(AllocId, ReachRole),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ReachRole {
    Deep,
    Shallow,
    Projected,
}

impl OperandGroup {
    fn visit_keys(self, state: &BorrowState, mut visit: impl FnMut(OperandIncidence)) {
        let mut each = |key| {
            count_operand_key();
            visit(key);
            Continue(())
        };
        use OperandIncidence as K;
        match self {
            Self::Cell(field, id, traversal) => {
                let _ = state.visit_sharing_incidence(id, |pair| {
                    let cell = CellIncidence::Projected(field, pair);
                    each(if traversal {
                        K::Traversal(cell)
                    } else {
                        K::Address(cell)
                    })
                });
            }
            Self::Share(id) => {
                let _ = state.visit_sharing_incidence(id, |pair| each(K::GraphShare(pair)));
            }
            Self::Reach(..) => unreachable!("reach uses direct graph links"),
        }
    }
}

/// Keeping root incidences separate from argument positions lets repeated operands share their
/// expansion. Search-side matches are also cached by root and access role.
#[derive(Default)]
struct ExpandedOperands {
    positions: FxHashMap<OperandGroup, SmallSet<usize>>,
    by_key: Option<FxHashMap<OperandIncidence, SmallSet<OperandGroup>>>,
    matches: FxHashMap<OperandGroup, Vec<OperandGroup>>,
}

impl ExpandedOperands {
    fn any_later(
        &mut self,
        state: &BorrowState,
        group: OperandGroup,
        after: usize,
        check: &mut impl FnMut(usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if self.positions.is_empty() {
            return Continue(());
        }
        let by_key = self.by_key.get_or_insert_with(|| {
            let mut by_key: FxHashMap<_, SmallSet<_>> = FxHashMap::default();
            let mut owners: Vec<_> = self.positions.keys().copied().collect();
            owners.sort_unstable();
            for owner in owners {
                owner.visit_keys(state, |key| {
                    by_key.entry(key).or_insert(SmallSet::Empty).insert(owner);
                });
            }
            by_key
        });
        let matches = self.matches.entry(group).or_insert_with(|| {
            let mut matches = Vec::new();
            let mut seen = FxHashSet::default();
            group.visit_keys(state, |key| {
                let _ = key.visit_matches(|key| {
                    if let Some(groups) = by_key.get(&key) {
                        for &owner in groups.as_slice() {
                            count_operand_posting();
                            if seen.insert(owner) {
                                matches.push(owner);
                            }
                        }
                    }
                    Continue(())
                });
            });
            matches.sort_unstable();
            matches
        });
        for &owner in matches.iter() {
            count_operand_posting();
            visit_later_positions(self.positions[&owner].as_slice(), after, &mut *check)?;
        }
        Continue(())
    }
}

fn count_operand_key() {
    increment_frontend_counter(FrontendCounter::BorrowOperandIndexKeys);
}

fn count_operand_posting() {
    increment_frontend_counter(FrontendCounter::BorrowOperandPostingRows);
}

fn visit_later_positions(
    positions: &[usize],
    after: usize,
    mut check: impl FnMut(usize) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let start = positions.partition_point(|&index| {
        count_operand_posting();
        index <= after
    });
    for &index in &positions[start..] {
        count_operand_posting();
        check(index)?;
    }
    Continue(())
}

#[derive(Default)]
struct OperandReach {
    roots: FxHashMap<(AllocId, ReachRole), SmallSet<usize>>,
    // Paths to captured operands can remain implicit in storage, even if other fields are explicit.
    paths: Option<BTreeSet<(AllocId, AllocId)>>,
    has_deep: bool,
}

#[derive(Default)]
struct ReachWalk {
    seen: FxHashSet<AllocId>,
    pending: Vec<AllocId>,
}

impl ReachWalk {
    fn push(&mut self, id: AllocId) -> bool {
        let unseen = self.seen.insert(id);
        if unseen {
            self.pending.push(id);
        }
        unseen
    }
}

impl OperandReach {
    fn insert(&mut self, position: usize, root: AllocId, role: ReachRole) {
        self.has_deep |= role == ReachRole::Deep;
        self.roots
            .entry((root, role))
            .or_insert(SmallSet::Empty)
            .insert(position);
    }

    fn check_root(
        roots: &FxHashMap<(AllocId, ReachRole), SmallSet<usize>>,
        id: AllocId,
        roles: &[ReachRole],
        after: usize,
        check: &mut impl FnMut(usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        for &role in roles {
            count_operand_key();
            if let Some(postings) = roots.get(&(id, role)) {
                visit_later_positions(postings.as_slice(), after, &mut *check)?;
            }
        }
        Continue(())
    }

    fn check_reverse(
        &self,
        state: &BorrowState,
        root: AllocId,
        walk: &mut ReachWalk,
        after: usize,
        check: &mut impl FnMut(usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let mut discover = |id, walk: &mut ReachWalk| {
            if walk.push(id) {
                Self::check_root(&self.roots, id, &[ReachRole::Deep], after, check)?;
            }
            Continue(())
        };
        discover(root, walk)?;
        while let Some(id) = walk.pending.pop() {
            state.visit_operand_reach(id, true, |next| discover(next, walk))?;
        }
        Continue(())
    }

    fn any_later(
        &mut self,
        state: &BorrowState,
        probe: &AccessFootprint,
        after: usize,
        check: &mut impl FnMut(usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if self.roots.is_empty() {
            return Continue(());
        }
        let mut forward = ReachWalk::default();
        let mut reverse = ReachWalk::default();
        let roles = [ReachRole::Deep, ReachRole::Shallow, ReachRole::Projected];
        OperandAccessIndex::visit_groups(probe, |group| {
            if let OperandGroup::Reach(root, role) = group {
                count_operand_key();
                if role == ReachRole::Deep && forward.push(root) {
                    Self::check_root(&self.roots, root, &roles, after, check)?;
                }
                if self.has_deep {
                    self.check_reverse(state, root, &mut reverse, after, check)?;
                }
            }
            Continue(())
        })?;
        while let Some(id) = forward.pending.pop() {
            if self.has_deep {
                // A reverse search at an intersection finds the later operands without
                // expanding every later root or assigning its own copy of a shared tail.
                self.check_reverse(state, id, &mut reverse, after, check)?;
            }
            state.visit_operand_reach(id, false, |next| {
                if forward.push(next) {
                    Self::check_root(&self.roots, next, &roles, after, check)?;
                }
                Continue(())
            })?;
        }
        self.check_implicit_paths(state, forward, &roles, after, check)
    }

    fn check_implicit_paths(
        &mut self,
        state: &BorrowState,
        walk: ReachWalk,
        roles: &[ReachRole],
        after: usize,
        check: &mut impl FnMut(usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let ReachWalk {
            mut seen,
            pending: mut lazy,
        } = walk;
        if seen.is_empty() {
            return Continue(());
        }

        // Only paths to captured targets can add further destinations after walking
        // the explicit graph. Build those postings once, and only if needed.
        let paths = self.paths.get_or_insert_with(|| {
            let mut paths = BTreeSet::new();
            for &(root, _) in self.roots.keys() {
                state.visit_operand_path(root, |parent, child| paths.insert((parent, child)));
            }
            paths
        });
        let push_children = |id, pending: &mut Vec<_>| {
            pending.extend(
                paths
                    .range((id, AllocId::MIN)..=(id, AllocId::MAX))
                    .map(|&(_, child)| child),
            );
        };
        for &id in &seen {
            push_children(id, &mut lazy);
        }
        while let Some(id) = lazy.pop() {
            if !seen.insert(id) || !state.operand_lazy_child(id) {
                continue;
            }
            Self::check_root(&self.roots, id, roles, after, check)?;
            push_children(id, &mut lazy);
        }
        Continue(())
    }
}

/// Exact identity and incomplete coverage can settle a query before any graph expansion.
/// An indexed candidate is still checked by the same typed overlap predicate.
pub(super) struct OperandAccessIndex {
    postings: FxHashMap<OperandIncidence, SmallSet<usize>>,
    sharing: ExpandedOperands,
    reach: OperandReach,
    seen: Vec<usize>,
}

impl OperandAccessIndex {
    pub(super) fn new<'a>(
        len: usize,
        operands: impl Iterator<Item = (usize, &'a AccessFootprint)>,
    ) -> Self {
        let mut index = Self {
            postings: FxHashMap::default(),
            sharing: ExpandedOperands::default(),
            reach: OperandReach::default(),
            seen: vec![usize::MAX; len],
        };
        for (position, access) in operands {
            let _ = Self::visit_direct(access, |key| {
                index
                    .postings
                    .entry(key)
                    .or_insert(SmallSet::Empty)
                    .insert(position);
                Continue(())
            });
            let _ = Self::visit_groups(access, |group| {
                count_operand_key();
                if let OperandGroup::Reach(root, role) = group {
                    index.reach.insert(position, root, role);
                } else {
                    index
                        .sharing
                        .positions
                        .entry(group)
                        .or_insert(SmallSet::Empty)
                        .insert(position);
                }
                Continue(())
            });
        }
        index
    }

    pub(super) fn any_later(
        &mut self,
        state: &BorrowState,
        after: usize,
        probe: &AccessFootprint,
        mut check: impl FnMut(usize) -> bool,
    ) -> bool {
        let seen = &mut self.seen;
        let mut check = |index| {
            if seen[index] != after {
                seen[index] = after;
                if check(index) {
                    return Break(());
                }
            }
            Continue(())
        };
        if Self::visit_direct(probe, |key| {
            key.visit_matches(|key| {
                if let Some(positions) = self.postings.get(&key) {
                    visit_later_positions(positions.as_slice(), after, &mut check)?;
                }
                Continue(())
            })
        })
        .is_break()
        {
            return true;
        }
        if Self::visit_groups(probe, |group| {
            count_operand_key();
            if matches!(group, OperandGroup::Reach(..)) {
                return Continue(());
            }
            self.sharing.any_later(state, group, after, &mut check)
        })
        .is_break()
        {
            return true;
        }
        self.reach
            .any_later(state, probe, after, &mut check)
            .is_break()
    }

    #[cfg(test)]
    pub(super) fn reach_work(&self) -> (usize, bool) {
        (self.reach.roots.len(), self.reach.paths.is_some())
    }

    fn visit_direct(
        access: &AccessFootprint,
        mut visit: impl FnMut(OperandIncidence) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let mut each = |key| {
            count_operand_key();
            visit(key)
        };
        use OperandIncidence::*;
        if let Some(graph) = GraphFootprint::of(access) {
            for &id in graph.share_ids() {
                each(GraphShare((id, id)))?;
            }
            if graph.coverage == Coverage::Partial {
                each(GraphPartial)?;
            }
            if !graph.ids.is_empty() || graph.coverage == Coverage::Partial {
                each(GraphPossible)?;
            }
            if matches!(access.usage, PlaceUse::ReadEntries | PlaceUse::WriteEntries) {
                for &id in graph.ids {
                    each(EntryReceiver(id))?;
                }
            }
        }
        for (cells, wrap, coverage, possible, partial) in [
            (
                access.address.cells(),
                Address as fn(CellIncidence) -> OperandIncidence,
                access.address.may_coverage(),
                AddressPossible,
                AddressPartial,
            ),
            (
                access.traversal.as_slice(),
                Traversal as fn(CellIncidence) -> OperandIncidence,
                access.traversal_coverage,
                TraversalPossible,
                TraversalPartial,
            ),
        ] {
            for &cell in cells {
                let cell = match cell {
                    CellId::Slot(slot) => CellIncidence::Slot(slot),
                    CellId::Projected(holder, field) => {
                        CellIncidence::Projected(field, (holder, holder))
                    }
                };
                each(wrap(cell))?;
            }
            if coverage == Coverage::Partial {
                each(partial)?;
            }
            if !cells.is_empty() || coverage == Coverage::Partial {
                each(possible)?;
            }
        }
        if writable(access) && access.usage.touches_cell() {
            for &cell in access.address.cells() {
                if let CellId::Projected(holder, field) = cell {
                    each(ProjectedEdit)?;
                    if touches_entries(field) {
                        each(EntryEdit(holder))?;
                    }
                }
            }
        }
        Continue(())
    }

    fn visit_groups(
        access: &AccessFootprint,
        mut each: impl FnMut(OperandGroup) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        use OperandGroup::*;
        if let Some(graph) = GraphFootprint::of(access) {
            for &id in graph.share_ids() {
                each(Share(id))?;
            }
            let role = if graph.deep() {
                ReachRole::Deep
            } else {
                ReachRole::Shallow
            };
            for &id in graph.ids {
                each(Reach(id, role))?;
            }
        }
        for &cell in access.address.cells() {
            if let CellId::Projected(holder, field) = cell {
                each(Cell(field, holder, false))?;
                if writable(access) && access.usage.touches_cell() {
                    each(Reach(holder, ReachRole::Projected))?;
                }
            }
        }
        for &cell in access.traversal.as_slice() {
            if let CellId::Projected(holder, field) = cell {
                each(Cell(field, holder, true))?;
            }
        }
        Continue(())
    }
}

pub(in super::super::super) fn overlap(
    state: &BorrowState,
    a: &AccessFootprint,
    b: &AccessFootprint,
) -> Overlap {
    overlap_with_detachment(state, a, b, false)
}

fn overlap_with_detachment(
    state: &BorrowState,
    a: &AccessFootprint,
    b: &AccessFootprint,
    detached: bool,
) -> Overlap {
    increment_frontend_counter(FrontendCounter::BorrowOverlapCalls);
    if !writable(a) && !writable(b) {
        return Overlap::No;
    }
    if let (Some(a), Some(b)) = (a.implicit_slot(), b.implicit_slot()) {
        return if a == b { Overlap::Yes } else { Overlap::No };
    }
    let mut result = Overlap::No;
    let mut consider = |relation| {
        if relation == Overlap::Unknown {
            result = Overlap::Unknown;
        }
        relation == Overlap::Yes
    };
    if ((writable(a) && a.usage.touches_cell()) || (writable(b) && b.usage.touches_cell()))
        && consider(cell_relation(
            state,
            a.address.cells(),
            a.address.may_coverage(),
            b.address.cells(),
            b.address.may_coverage(),
        ))
    {
        return Overlap::Yes;
    }
    for (writer, reader) in [(a, b), (b, a)] {
        if writable(writer)
            && writer.usage.touches_cell()
            && consider(cell_relation(
                state,
                writer.address.cells(),
                writer.address.may_coverage(),
                reader.traversal.as_slice(),
                reader.traversal_coverage,
            ))
        {
            return Overlap::Yes;
        }
        if let Some(graph) = GraphFootprint::of(reader)
            && consider(projected_edit_overlaps(state, writer, graph))
        {
            return Overlap::Yes;
        }
    }
    if !detached && consider(graph_relation(state, a, b)) {
        return Overlap::Yes;
    }
    result
}

pub(in super::super::super) fn footprint(
    resolved: &PlaceResolution,
    usage: PlaceUse,
) -> AccessFootprint {
    AccessFootprint {
        address: resolved.address.clone(),
        traversal: resolved.traversal.clone(),
        traversal_coverage: resolved.traversal_coverage,
        content: resolved.content.clone(),
        usage,
    }
}

fn into_footprint(resolved: PlaceResolution, usage: PlaceUse) -> AccessFootprint {
    AccessFootprint {
        address: resolved.address,
        traversal: resolved.traversal,
        traversal_coverage: resolved.traversal_coverage,
        content: resolved.content,
        usage,
    }
}

pub(super) fn check_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    access: AccessFootprint,
    source: Option<&PlaceResolution>,
    policy: MutableAccessPolicy,
) -> Result<(), BorrowCheckError> {
    validate_access(check, &access, source, policy)?;
    if check.state.has_dependency_carriers() {
        let terminal = matches!(
            access.usage,
            PlaceUse::Read | PlaceUse::ReadEntries | PlaceUse::WriteEntries
        );
        check.stats.observations.tail.observe(
            check.state,
            &access,
            terminal,
            check.actor_index_hint,
        );
        if let Some(checkpoint) = mutation_checkpoint(
            check.state,
            &access,
            check.actor_index_hint,
            check.span,
            check.current_order,
            None,
        ) {
            check
                .stats
                .observations
                .split(ObservationBoundary::Checkpoint(checkpoint));
        }
    }
    publish_access(check, roots, access);
    Ok(())
}

/// Call reservations are validated individually, but the call executes after all operands.
pub(super) fn check_call_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    access: AccessFootprint,
    source: Option<&PlaceResolution>,
) -> Result<(), BorrowCheckError> {
    validate_access(check, &access, source, BORROW_ACCESS)?;
    publish_access(check, roots, access);
    Ok(())
}

pub(super) fn record_call_observations(
    observations: &mut ValueObservations,
    state: &BorrowState,
    tracker: &StatementAccessTracker,
    order: i32,
    call_span: Option<SourceSpan>,
    user_call: bool,
    defer_writes: bool,
) -> Vec<ObservationCheckpoint> {
    if !state.has_dependency_carriers() {
        return Vec::new();
    }
    let mut deferred = Vec::new();
    for reservation in tracker.reservations() {
        let terminal = !writable(&reservation.footprint)
            || matches!(reservation.footprint.usage, PlaceUse::WriteEntries);
        observations
            .tail
            .observe(state, &reservation.footprint, terminal, reservation.actor);
    }
    for reservation in tracker.reservations() {
        if let Some(checkpoint) = mutation_checkpoint(
            state,
            &reservation.footprint,
            reservation.actor,
            if user_call {
                call_span.or(reservation.span)
            } else {
                reservation.span
            },
            order,
            Some(tracker),
        ) {
            if defer_writes {
                deferred.push(checkpoint);
            } else {
                observations.split(ObservationBoundary::Checkpoint(checkpoint));
            }
        }
    }
    if user_call {
        // The contract need not contain a store or result for the callee to read an entry
        // value. Its read can follow any of the call's possible writes.
        for reservation in tracker.reservations().filter(|r| writable(&r.footprint)) {
            observations
                .tail
                .observe(state, &reservation.footprint, true, reservation.actor);
        }
    }
    deferred
}

fn mutation_checkpoint(
    state: &BorrowState,
    access: &AccessFootprint,
    actor: Option<usize>,
    span: Option<SourceSpan>,
    order: i32,
    pending: Option<&StatementAccessTracker>,
) -> Option<ObservationCheckpoint> {
    if !state.has_dependency_carriers()
        || !matches!(
            access.usage,
            PlaceUse::WriteEntries | PlaceUse::Mutate { deep: true }
        ) && !(matches!(access.usage, PlaceUse::Replace | PlaceUse::Mutate { .. })
            && (access.address.may_coverage() == Coverage::Partial
                || access.address.cells().iter().any(
                    |cell| matches!(*cell, CellId::Projected(_, key) if touches_entries(key)),
                )))
    {
        return None;
    }
    let graph = GraphFootprint::of(access);
    let candidates: Demand = matching_carriers(state, |deps| {
        deps.iter().any(|dependency| {
            let receiver = GraphFootprint {
                ids: dependency.entries.as_slice(),
                views: &[],
                coverage: dependency.coverage,
                usage: PlaceUse::ReadEntries,
            };
            projected_edit_overlaps(state, access, receiver) != Overlap::No
                || graph.is_some_and(|graph| graph_overlap(state, graph, receiver) != Overlap::No)
        })
    })
    .collect();
    if candidates.is_empty() {
        return None;
    }
    let mut reads = Demand::default();
    if let Some(tracker) = pending {
        for reservation in tracker.reservations().filter(|r| !writable(&r.footprint)) {
            visit_observed_carriers(
                state,
                &reservation.footprint,
                true,
                reservation.actor,
                |carrier| {
                    if candidates.contains(&carrier) {
                        reads.insert(carrier);
                    }
                },
            );
        }
    }
    Some(ObservationCheckpoint {
        order,
        actor,
        span,
        candidates,
        pending: reads,
    })
}

/// A successful optimisation probe publishes the checked reservation; a failed probe
/// never mutates statement facts or emits its transfer-only diagnostic.
pub(super) fn probe_mutable_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    mut reservation: AccessFootprint,
    source: Option<&PlaceResolution>,
    policy: MutableAccessPolicy,
) -> Result<Option<AccessFootprint>, BorrowCheckError> {
    let usage = reservation.usage;
    reservation.usage = usage.for_transfer_probe();
    let result = validate_access(check, &reservation, source, policy);
    reservation.usage = usage;
    match result {
        Ok(()) => {
            // The optional transfer is exclusive, but the callee receives only its declared access.
            check.tracker.report(roots, AccessKind::Mutable);
            publish_access(check, roots, reservation);
            Ok(None)
        }
        Err(BorrowCheckError::Diagnostic(_)) => Ok(Some(reservation)),
        Err(error) => Err(error),
    }
}

fn publish_access(
    check: &mut AccessCheckContext<'_, '_>,
    roots: &RootSet,
    footprint: AccessFootprint,
) {
    check.stats.conflicts_checked += 1;
    check.tracker.report(roots, footprint.usage.kind());
    check.tracker.reserve(AccessReservation {
        footprint,
        actor: check.actor_index_hint,
        span: check.span,
    });
}

fn validate_access(
    check: &AccessCheckContext<'_, '_>,
    access: &AccessFootprint,
    source: Option<&PlaceResolution>,
    policy: MutableAccessPolicy,
) -> Result<(), BorrowCheckError> {
    let actor = check.actor_index_hint;
    let kind = access.usage.kind();
    let mutable = kind == AccessKind::Mutable;
    let activity = AliasActivityContext {
        layout: check.layout,
        block_id: check.block_id,
        current_order: check.current_order,
        preserve_cfg_future_use_activity: preserves_cfg_future_use_activity(
            check.context,
            check.layout,
            actor,
        ),
    };
    let diagnostics = &check.context.diagnostics;
    let describe = |index: usize| diagnostics.local_place(check.layout.local_ids[index]);
    let place = || actor.map_or(DiagnosticPlace::Unknown, describe);
    let reject_mutation = |candidate: usize| {
        diagnostics.invalid_mutable_access(
            place(),
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
            Some(describe(candidate)),
            diagnostics.local_source_span(check.layout.local_ids[candidate]),
            check.span,
        )
    };
    let old_projection = OnceCell::new();
    let derivation = source
        .filter(|_| !policy.strict_move_exclusivity)
        .map(|place| check.state.binding(place.route.local).derivation());

    let candidate_conflicts = |local, wrapped, mode| {
        if Some(local) == actor
            || mode == LocalMode::UNINIT
            || derivation.is_some_and(|roots| roots.contains(local))
        {
            return false;
        }
        let source_alias = wrapped
            && check.layout.local_mutable[local]
            && !matches!(diagnostics.local_origin_kind(check.layout.local_ids[local]),
                Some(kind) if kind != HirLocalOriginKind::User);
        if !mutable && !source_alias {
            return false;
        }
        let active = is_local_active_for_alias_conflict(
            &activity,
            local,
            wrapped,
            source_alias,
            policy.strict_move_exclusivity,
        ) || source_alias && !mutable && local_alias_never_read(check.layout, local);
        if !active && (!policy.strict_move_exclusivity || !wrapped) {
            return false;
        }
        let mut resolved = check
            .state
            .resolve_place_path(local, &check.layout.local_mutable, []);
        if !active && !resolved.address.cells().iter().any(|cell| {
            matches!(cell, CellId::Slot(WrapperId { age: WrapperAge::Current, local: owner }) if *owner != local)
        })
        {
            return false;
        }
        let slot = WrapperId::current(local);
        let owns_slot = !wrapped && resolved.address.definite() == Some(CellId::Slot(slot));
        let terminal_owner =
            owns_slot && source.is_some_and(|place| place.names_terminal_slot(slot));
        if terminal_owner && access.address.definite() == Some(CellId::Slot(slot)) {
            return false;
        }
        let usage = if source_alias {
            PlaceUse::Mutate { deep: true }
        } else {
            PlaceUse::Read
        };
        if check.layout.local_has_in_place_mutation_surface[local] {
            check
                .state
                .load_place_content(&mut resolved, &check.layout.local_mutable, false);
        }
        if owns_slot
            && source.and_then(PlaceResolution::projection_owner) == Some(slot)
            && let Some(content) = &resolved.content
            && !access.address.cells().is_empty()
            && access.address.cells().iter().all(|cell| {
                matches!(cell, CellId::Projected(holder, _) if
                check.state.alloc_reaches_typed(&content.ids, *holder, true, false))
            })
        {
            // A writer uses this owner's captured cell, not an independent observation.
            return false;
        }
        let holder_route = if !mutable
            && let Some(actor) = actor
            && source.is_some_and(|source| {
                source.route.projected
                    || source.address.cells().iter().any(|cell| {
                        matches!(cell, CellId::Slot(slot)
                            if !slot.is_call_owned() && slot.local == actor)
                    })
            }) {
            resolved
                .projected_holder_route(actor)
                .map(|isolated| (actor, isolated))
        } else {
            None
        };
        let mut candidate = into_footprint(resolved, usage);
        if terminal_owner {
            // Exempt this slot alternative only. The owner's content may still observe a
            // different projected write alternative of a joined destination.
            candidate.address = AddressSet::from_cells([], Coverage::Complete);
        }
        if let Some((actor, isolated)) = holder_route
            && !holder_routes_separate(check.state, access, &candidate, actor, isolated)
        {
            return true;
        }
        let detached = access
            .content
            .as_ref()
            .zip(candidate.content.as_ref())
            .is_some_and(|(a, b)| {
                actor.is_some_and(|actor| check.state.detached_from(actor, &b.ids))
                    || check.state.detached_from(local, &a.ids)
            });
        if matches!(access.usage, PlaceUse::Replace)
            && wrapped
            && access.address.cells().iter().any(|cell| {
                matches!(
                    cell,
                    CellId::Projected(_, FieldKey::Field(_) | FieldKey::Payload { .. })
                )
            })
            && candidate.address.cells().iter().any(|cell| {
                matches!(
                    cell,
                    CellId::Projected(_, FieldKey::Field(_) | FieldKey::Payload { .. })
                )
            })
            && let Some(source) = source
            && !detached
        {
            // A retained field route can observe a shared old child. Replacing the cell
            // does not write that child or conflict with an independently acquired value.
            let old = old_projection.get_or_init(|| {
                let mut old = source.clone();
                check
                    .state
                    .load_place_content(&mut old, &check.layout.local_mutable, false);
                into_footprint(old, PlaceUse::Read)
            });
            if graph_relation(check.state, old, &candidate) != Overlap::No {
                return true;
            }
        }
        overlap_with_detachment(check.state, access, &candidate, detached) != Overlap::No
    };
    let mut conflicting = check.state.wrapper_locals().find(|&local| {
        if !mutable && !check.layout.local_mutable[local] {
            return false;
        }
        let mode = check.state.binding_mode(local);
        let alias = mode.contains(LocalMode::ALIAS);
        (mutable || alias) && candidate_conflicts(local, alias, mode)
    });
    if mutable
        && (access.content.is_some()
            || access
                .address
                .cells()
                .iter()
                .any(|cell| matches!(cell, CellId::Projected(..))))
    {
        let end = conflicting.unwrap_or(check.layout.local_count());
        let graph_edges = check.state.has_alloc_edges();
        let mut origins = SmallSet::Empty;
        let mut unknown_origin = false;
        let mut record_origin = |id| match check.state.alloc_origin_local(id) {
            Some(local) => origins.insert(local),
            None => unknown_origin = true,
        };
        if let Some(content) = &access.content {
            content.ids.iter().copied().for_each(&mut record_origin);
            if deep(access) && graph_edges {
                check
                    .state
                    .alloc_writable_closure(&content.ids)
                    .into_iter()
                    .for_each(&mut record_origin);
            }
        }
        for &cell in access.address.cells() {
            if let CellId::Projected(holder, _) = cell {
                record_origin(holder);
            }
        }
        let plain = |local| {
            if Some(local) == actor {
                return false;
            }
            let roots = check.state.binding(local).value_roots();
            let matching_origin = if roots.is_empty() {
                origins.as_slice().contains(&local)
            } else {
                graph_edges
                    || unknown_origin
                    || origins
                        .as_slice()
                        .iter()
                        .any(|&origin| roots.contains(origin))
            };
            matching_origin
                && !check.state.has_wrapped_binding(local)
                && candidate_conflicts(local, false, check.state.binding_mode(local))
        };
        conflicting = (0..end).find(|&local| plain(local)).or(conflicting);
        if let Some(local) = check
            .state
            .allocation_observer_locals()
            .filter(|&local| {
                local < conflicting.unwrap_or(check.layout.local_count())
                    && !check.state.has_wrapped_binding(local)
                    && candidate_conflicts(local, false, check.state.binding_mode(local))
            })
            .min()
        {
            conflicting = Some(local);
        }
    }
    if let Some(candidate) = conflicting {
        if !mutable
            || (!policy.strict_move_exclusivity
                && activity.preserve_cfg_future_use_activity
                && has_cfg_future_use_after_linear_expiry(
                    check.layout,
                    candidate,
                    check.block_id,
                    check.current_order,
                )
                && is_shared_alias_conflict(check.context, check.layout, candidate))
        {
            return Err(diagnostics.shared_mutable_conflict(
                place(),
                if mutable {
                    BorrowAccessKind::Shared
                } else {
                    BorrowAccessKind::Mutable
                },
                borrow_access_kind(kind),
                Some(describe(candidate)),
                None,
                check.span,
            ));
        }
        return Err(reject_mutation(candidate));
    }
    if let Some(existing) = check.tracker.conflict(access, |existing| {
        !(policy.allow_prior_shared && !writable(&existing.footprint))
            && overlap(check.state, access, &existing.footprint) != Overlap::No
    }) {
        let prior = existing.footprint.usage.kind();
        if mutable && prior == AccessKind::Mutable {
            return Err(diagnostics.multiple_mutable_borrows(
                place(),
                existing.actor.map(describe),
                existing.span,
                check.span,
            ));
        }
        return Err(diagnostics.shared_mutable_conflict(
            place(),
            borrow_access_kind(prior),
            borrow_access_kind(kind),
            existing.actor.map(describe),
            existing.span,
            check.span,
        ));
    }
    Ok(())
}

/// A read through the recorded holder occurrence is conservative across a rebind unless
/// both the old capture and the new holder prove independent slot ownership.
fn holder_routes_separate(
    state: &BorrowState,
    read: &AccessFootprint,
    writer: &AccessFootprint,
    actor: usize,
    isolated: bool,
) -> bool {
    let [CellId::Projected(holder, _)] = read.address.cells() else {
        return false;
    };
    if !state.alloc_singular(*holder)
        || read.address.coverage != Coverage::Complete
        || writer.address.coverage != Coverage::Complete
    {
        return false;
    }
    let siblings = writer
        .address
        .cells()
        .iter()
        .all(|cell| matches!(cell, CellId::Projected(other, _) if holder == other));
    siblings
        || isolated
            && state.alloc_origin_local(*holder) == Some(actor)
            && !state.has_layer_binding(actor)
            && writer
                .address
                .cells()
                .iter()
                .all(|cell| matches!(cell, CellId::Projected(other, _) if holder != other))
}

fn borrow_access_kind(access: AccessKind) -> BorrowAccessKind {
    match access {
        AccessKind::Shared => BorrowAccessKind::Shared,
        AccessKind::Mutable => BorrowAccessKind::Mutable,
    }
}

struct AliasActivityContext<'a> {
    layout: &'a FunctionLayout,
    block_id: BlockId,
    current_order: i32,
    preserve_cfg_future_use_activity: bool,
}

fn is_local_active_for_alias_conflict(
    activity: &AliasActivityContext<'_>,
    local: usize,
    wrapped: bool,
    source_alias: bool,
    strict: bool,
) -> bool {
    if !strict && !wrapped {
        return activity.layout.future_use_kind(
            activity.block_id,
            local,
            activity.current_order - 1,
        ) != FutureUseKind::None;
    }
    let mutable = activity.layout.local_mutable[local];
    if activity.layout.local_last_use_order[local] < 0 {
        return mutable;
    }
    if !activity
        .layout
        .local_is_expired(local, activity.current_order)
        && (!activity.preserve_cfg_future_use_activity
            || activity.layout.future_use_kind(
                activity.block_id,
                local,
                activity.current_order - 1,
            ) != FutureUseKind::None)
    {
        return true;
    }
    if activity.preserve_cfg_future_use_activity
        && ((source_alias
            && activity
                .layout
                .local_has_future_assignment(activity.block_id, local))
            || has_cfg_future_use_after_linear_expiry(
                activity.layout,
                local,
                activity.block_id,
                activity.current_order,
            ))
    {
        return true;
    }
    strict && mutable
}

// WHAT: Limits CFG future-use retention to source-semantic access actors.
// WHY: compiler-temporary rebinding in template lowering is not a source mutation and must keep
//      its existing linear-expiry behaviour.
fn preserves_cfg_future_use_activity(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    actor_index_hint: Option<usize>,
) -> bool {
    actor_index_hint.is_none_or(|actor_index| {
        !matches!(
            context
                .diagnostics
                .local_origin_kind(layout.local_ids[actor_index]),
            Some(HirLocalOriginKind::CompilerTemp)
        )
    })
}

fn has_cfg_future_use_after_linear_expiry(
    layout: &FunctionLayout,
    local_index: usize,
    block_id: BlockId,
    current_order: i32,
) -> bool {
    layout.local_is_expired(local_index, current_order)
        && matches!(
            layout.future_use_kind(block_id, local_index, current_order),
            FutureUseKind::May | FutureUseKind::Must
        )
}

fn is_shared_alias_conflict(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    local_index: usize,
) -> bool {
    !layout.local_mutable[local_index]
        || matches!(
            context
                .diagnostics
                .local_origin_kind(layout.local_ids[local_index]),
            Some(HirLocalOriginKind::CompilerTemp)
        )
}

fn local_alias_never_read(layout: &FunctionLayout, local_index: usize) -> bool {
    let first_write = layout.local_first_write_order[local_index];
    let last_use = layout.local_last_use_order[local_index];
    first_write >= 0 && first_write == last_use
}
