use super::compaction::CompactOutcome;
use super::prepared::prepare_draft;
use super::store::{DiagnosticId, DiagnosticStore};
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;

pub(crate) struct CompactDiagnosticBagForTest {
    store: DiagnosticStore,
    entries: Vec<(DiagnosticId, TypeDisplaySnapshot)>,
}

impl CompactDiagnosticBagForTest {
    pub(crate) fn views(&self) -> impl Iterator<Item = DiagnosticView<'_>> + '_ {
        self.entries
            .iter()
            .map(|(id, types)| DiagnosticView::new(&self.store, *id, types))
    }

    pub(crate) fn first(&self) -> Option<DiagnosticView<'_>> {
        self.entries
            .first()
            .map(|(id, types)| DiagnosticView::new(&self.store, *id, types))
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub(crate) fn compact_bag_for_test(
    bag: DiagnosticBag,
    environment: Option<&TypeEnvironment>,
) -> CompactDiagnosticBagForTest {
    let mut store = DiagnosticStore::new();
    let mut entries = Vec::new();
    for draft in bag.into_diagnostics() {
        let prepared = prepare_draft(draft, environment);
        let id = match store.compact(prepared.draft) {
            CompactOutcome::Stored(id) => id,
            CompactOutcome::Rejected(_) => DiagnosticId::from_index(store.len() - 1),
        };
        entries.push((id, prepared.types));
    }
    CompactDiagnosticBagForTest { store, entries }
}
