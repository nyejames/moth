//! Shared fact collection and per-statement access tracking.
//!
//! These helpers are intentionally lightweight because they run for every
//! statement/terminator transfer.

use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, CellId, Coverage, FunctionLayout, ResolvedPlace, RootSet, SmallSet, WrapperAge,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    AccessKind, OptionalTransferStatus, ValueAccessClassification, ValueBorrowFact,
};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::source::SourceSpan;
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;

/// The terminal operation. A projection's prefixes are always observed, not replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlaceUse {
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
}

/// Cell identities and graph alternatives stay separate, including when one is empty.
#[derive(Debug)]
pub(super) struct AccessFootprint {
    pub address: AddressSet,
    pub traversal: SmallSet<CellId>,
    pub traversal_coverage: Coverage,
    pub content: Option<ResolvedPlace>,
    pub usage: PlaceUse,
}

impl AccessFootprint {
    pub(super) fn cells(address: AddressSet, usage: PlaceUse) -> Self {
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
