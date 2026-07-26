//! Shared fact collection and per-statement access tracking.
//!
//! These helpers are intentionally lightweight because they run for every
//! statement/terminator transfer.

use crate::compiler_frontend::analysis::borrow_checker::state::{FunctionLayout, RootSet};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    AccessKind, OptionalTransferStatus, ValueAccessClassification, ValueBorrowFact,
};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use rustc_hash::FxHashMap;

#[derive(Debug, Clone)]
pub(super) struct StatementAccessTracker {
    root_access: Vec<Option<AccessKind>>,
    // WHAT: first access location recorded per root, so same-statement conflicts can label
    // the earlier access without changing transfer rules.
    root_access_location: Vec<Option<SourceLocation>>,
    pub(super) shared_roots: RootSet,
    pub(super) mutable_roots: RootSet,
}

impl StatementAccessTracker {
    pub(super) fn new(root_count: usize) -> Self {
        Self {
            root_access: vec![None; root_count],
            root_access_location: vec![None; root_count],
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

    // WHAT: returns the source location of the first access recorded for this root.
    // WHY: same-statement conflicts report the earlier access as the secondary label.
    pub(super) fn access_location(&self, root_index: usize) -> Option<&SourceLocation> {
        self.root_access_location[root_index].as_ref()
    }

    pub(super) fn record(
        &mut self,
        root_index: usize,
        access: AccessKind,
        location: SourceLocation,
    ) {
        match access {
            AccessKind::Shared => self.shared_roots.insert(root_index),
            AccessKind::Mutable => self.mutable_roots.insert(root_index),
        }

        // Keep the first recorded location so the conflicting (earlier) access is labelled.
        if self.root_access_location[root_index].is_none() {
            self.root_access_location[root_index] = Some(location);
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
    facts: FxHashMap<HirValueId, (ValueAccessClassification, RootSet, OptionalTransferStatus)>,
}

impl ValueFactBuffer {
    pub(super) fn new(local_count: usize) -> Self {
        Self {
            local_count,
            facts: FxHashMap::default(),
        }
    }

    pub(super) fn record(
        &mut self,
        value_id: HirValueId,
        classification: ValueAccessClassification,
        roots: &RootSet,
    ) {
        let entry = self.facts.entry(value_id).or_insert_with(|| {
            (
                ValueAccessClassification::None,
                RootSet::empty(self.local_count),
                OptionalTransferStatus::NotAttempted,
            )
        });

        entry.0 = entry.0.merge(classification);
        entry.1.union_with(roots);
    }

    pub(super) fn record_optional_transfer(
        &mut self,
        value_id: HirValueId,
        status: OptionalTransferStatus,
        roots: &RootSet,
    ) {
        let entry = self.facts.entry(value_id).or_insert_with(|| {
            (
                ValueAccessClassification::None,
                RootSet::empty(self.local_count),
                OptionalTransferStatus::NotAttempted,
            )
        });

        entry.1.union_with(roots);
        entry.2 = entry.2.merge(status);
    }

    pub(super) fn into_serialized(
        self,
        layout: &FunctionLayout,
    ) -> Vec<(HirValueId, ValueBorrowFact)> {
        self.facts
            .into_iter()
            .map(|(value_id, (classification, roots, optional_transfer))| {
                (
                    value_id,
                    ValueBorrowFact {
                        classification,
                        roots: roots_to_local_ids(layout, &roots),
                        optional_transfer,
                    },
                )
            })
            .collect::<Vec<_>>()
    }
}

pub(super) fn roots_to_local_ids(layout: &FunctionLayout, roots: &RootSet) -> Vec<LocalId> {
    roots
        .iter_ones()
        .map(|index| layout.local_ids[index])
        .collect::<Vec<_>>()
}
