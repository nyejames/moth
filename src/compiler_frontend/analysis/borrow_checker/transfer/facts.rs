//! Shared fact collection and per-statement access tracking.
//!
//! These helpers are intentionally lightweight because they run for every
//! statement/terminator transfer.

use crate::compiler_frontend::analysis::borrow_checker::state::{FunctionLayout, RootSet};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    AccessKind, OptionalTransferStatus, ValueAccessClassification, ValueBorrowFact,
};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::source::SourceSpan;
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;

#[derive(Debug, Clone)]
pub(super) struct StatementAccessTracker {
    root_access: Vec<Option<AccessKind>>,
    // WHAT: first access span recorded per root, so same-statement conflicts can label
    // the earlier access without inventing spans for generated accesses.
    root_access_span: Vec<Option<SourceSpan>>,
    pub(super) shared_roots: RootSet,
    pub(super) mutable_roots: RootSet,
}

impl StatementAccessTracker {
    pub(super) fn new(root_count: usize) -> Self {
        Self {
            root_access: vec![None; root_count],
            root_access_span: vec![None; root_count],
            shared_roots: RootSet::empty(root_count),
            mutable_roots: RootSet::empty(root_count),
        }
    }

    pub(super) fn conflict(&self, root_index: usize, new_access: AccessKind) -> Option<AccessKind> {
        let existing = self.root_access[root_index]?;

        match (existing, new_access) {
            (AccessKind::Shared, AccessKind::Shared) => None,
            (AccessKind::Shared, AccessKind::Mutable)
            | (AccessKind::Mutable, AccessKind::Shared)
            | (AccessKind::Mutable, AccessKind::Mutable) => Some(existing),
        }
    }

    // WHAT: returns the span of the first access recorded for this root.
    // WHY: same-statement conflicts report the earlier access span without inventing
    // spans for generated accesses.
    pub(super) fn access_span(&self, root_index: usize) -> Option<SourceSpan> {
        self.root_access_span[root_index]
    }

    pub(super) fn record(
        &mut self,
        root_index: usize,
        access: AccessKind,
        span: Option<SourceSpan>,
    ) {
        match access {
            AccessKind::Shared => self.shared_roots.insert(root_index),
            AccessKind::Mutable => self.mutable_roots.insert(root_index),
        }

        // Keep the first recorded span so the conflicting earlier access is labelled.
        if self.root_access_span[root_index].is_none() {
            self.root_access_span[root_index] = span;
        }

        let entry = &mut self.root_access[root_index];
        match (*entry, access) {
            (Some(AccessKind::Mutable), _) => {}
            (_, AccessKind::Mutable) => *entry = Some(AccessKind::Mutable),
            (None, AccessKind::Shared) => *entry = Some(AccessKind::Shared),
            (Some(AccessKind::Shared), AccessKind::Shared) => {}
        }
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
