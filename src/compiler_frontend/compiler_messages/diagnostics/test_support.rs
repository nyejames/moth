use super::compaction::CompactOutcome;
use super::draft::DiagnosticDraft;
use super::families::{imports, project_config, token_lexical};
use super::prepared::prepare_draft;
use super::store::{DiagnosticId, DiagnosticStore};
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PathKind};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceFailure;

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

/// Rebuilds the draft that a Stage 0 file-reference failure becomes, through the same family
/// constructors the production emitters call.
fn resolved_failure_draft_for_test(failure: ResolvedFileReferenceFailure) -> DiagnosticDraft {
    match failure {
        ResolvedFileReferenceFailure::InvalidCompileTimePath { path, reason, span } => {
            project_config::invalid_compile_time_path(path, reason, Some(span))
        }
        ResolvedFileReferenceFailure::InvalidComponent { span } => {
            token_lexical::invalid_path(PathKind::InvalidComponent, Some(span))
        }
        ResolvedFileReferenceFailure::UnsupportedSourceFileKind {
            path,
            extension,
            span,
        } => imports::unsupported_source_file_kind(path, extension, Some(span)),
    }
}

/// Compacts one Stage 0 file-reference failure as the diagnostic it becomes.
pub(crate) fn compact_resolved_failure_for_test(
    failure: ResolvedFileReferenceFailure,
) -> CompactDiagnosticBagForTest {
    compact_bag_for_test(
        DiagnosticBag::from(resolved_failure_draft_for_test(failure)),
        None,
    )
}
