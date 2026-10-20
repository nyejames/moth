//! Compacts one legacy diagnostic into a store, so its family renderer can read it.
//!
//! WHAT: a pilot diagnostic is mapped onto its family draft, given its severity, labels and foreign sites, and its
//! types are captured as display ids. The draft is then stored and read back through a typed view. A diagnostic
//! outside the pilot families, or with a label the compact model cannot carry, is declined, and the caller keeps the
//! legacy payload path. A draft that faults while it is filled, or that the store refuses, is stored as its capacity
//! record, which renders as a compiler limit.
//! WHY: rendering switches to compact records one diagnostic at a time. A declined diagnostic renders exactly as
//! before, so its output does not change while producers still build legacy payloads. Each call builds a store for
//! its one diagnostic; a shared store replaces it once the boundary owns one.

use super::compaction::CompactOutcome;
use super::draft::DiagnosticDraft;
use super::families;
use super::foreign::{ForeignSiteDraft, ForeignSlot};
use super::store::{DiagnosticId, DiagnosticStore};
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    type_display_snapshot::TypeDisplaySnapshot,
    type_display_snapshot_builder::TypeDisplaySnapshotBuilder,
};
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;

/// One compacted diagnostic, with the store and the type snapshot that its record reads from.
pub(crate) struct CompactRender {
    store: DiagnosticStore,
    id: DiagnosticId,
    types: TypeDisplaySnapshot,
}

impl CompactRender {
    /// The typed view of the stored record.
    pub(crate) fn view(&self) -> DiagnosticView<'_> {
        DiagnosticView::new(&self.store, self.id, &self.types)
    }
}

/// Compacts `diagnostic` when its family is a pilot and the compact model carries it faithfully.
///
/// Returns `None` for every other diagnostic. The caller then keeps the legacy payload path.
pub(crate) fn compact_for_render(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Option<CompactRender> {
    let mut draft = finish(families::from_legacy(diagnostic)?, diagnostic)?;

    // Spellings resolve only when the live environment and the path table both exist, which is the condition
    // `diagnostic_type_name` applies. Any other type becomes an unresolved root that prints its raw id.
    let environment = context
        .type_environment
        .filter(|_| context.path_table().is_some());
    let mut builder = TypeDisplaySnapshotBuilder::new(environment);
    draft
        .capture_types(&mut |type_id| builder.display_top_level(type_id))
        .expect("one diagnostic's types always fit the u32 display-id space");
    let types = builder.finish();

    let mut store = DiagnosticStore::new();
    // A refused draft latches the fresh store, whose only record is then its capacity record at index 0.
    let id = match store.compact(draft) {
        CompactOutcome::Stored(id) => id,
        CompactOutcome::Rejected(_) => DiagnosticId::from_index(0),
    };
    Some(CompactRender { store, id, types })
}

/// Adds the legacy severity, labels and foreign sites to a family draft.
///
/// Declines a label without a message, or a label role the family does not declare. Compaction asserts the role,
/// so it is checked here rather than left to panic. A faulted draft is kept: it already is the capacity record.
fn finish(mut draft: DiagnosticDraft, diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    draft = draft.with_severity(diagnostic.severity);
    for label in &diagnostic.labels {
        let message = label.message.clone()?;
        draft = draft.with_label(label.span, message);
    }
    if !draft.labels_match_family() {
        return None;
    }

    for (index, label) in diagnostic.labels.iter().enumerate() {
        if let Some(handle) = &label.frozen_identity_handle {
            draft = draft.with_foreign(ForeignSiteDraft {
                slot: ForeignSlot::Label(index),
                context: handle.clone(),
            });
        }
    }
    if let Some(handle) = &diagnostic.primary_frozen_identity_handle {
        draft = draft.with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        });
    }

    Some(draft)
}
