use super::boundary_accumulator::BoundaryAccumulator;
use super::compaction::CompactOutcome;
use super::draft::DiagnosticDraft;
use super::families::{imports, project_config, token_lexical};
use super::freeze::InstallMode;
use super::prepared::prepare_draft;
use super::store::{DiagnosticId, DiagnosticStore};
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PathKind};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceFailure;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, LocalSpan, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

pub(crate) struct CompactDiagnosticBagForTest {
    store: DiagnosticStore,
    foreign_contexts: Vec<Option<Arc<FrozenIdentityContext>>>,
    entries: Vec<(DiagnosticId, TypeDisplaySnapshot)>,
}

impl CompactDiagnosticBagForTest {
    pub(crate) fn views(&self) -> impl Iterator<Item = DiagnosticView<'_>> + '_ {
        self.entries
            .iter()
            .map(|(id, types)| DiagnosticView::new(&self.store, *id, types, &self.foreign_contexts))
    }

    pub(crate) fn first(&self) -> Option<DiagnosticView<'_>> {
        self.entries
            .first()
            .map(|(id, types)| DiagnosticView::new(&self.store, *id, types, &self.foreign_contexts))
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
    let mut foreign_contexts = Vec::new();
    let mut entries = Vec::new();
    for draft in bag.into_diagnostics() {
        let prepared = prepare_draft(draft, environment);
        let id = match store.compact(prepared.draft, &mut foreign_contexts) {
            CompactOutcome::Stored(id) => id,
            CompactOutcome::Rejected(_) => DiagnosticId::from_index(store.len() - 1),
        };
        entries.push((id, prepared.types));
    }
    CompactDiagnosticBagForTest {
        store,
        foreign_contexts,
        entries,
    }
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

pub(crate) fn exact_span_in_source(
    source: SourceId,
    start: u32,
    length: u32,
    builder: &mut ExtendedSpanBuilder,
) -> SourceSpan {
    SourceSpan::new(
        source,
        LocalSpan::exact(start, length, builder).expect("test span should fit"),
    )
}

pub(crate) fn capture_rebased_batch(
    drafts: Vec<DiagnosticDraft>,
    local_table: StringTable,
    root_table: &mut StringTable,
    environment: Option<&TypeEnvironment>,
    path_table: Option<Arc<PathTable>>,
) -> PreparedDiagnosticBatch {
    let remap = root_table.merge_from(&local_table);
    assert!(
        !remap.is_identity(),
        "the fixture must exercise a shifting string-table remap"
    );
    let mut prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(drafts),
        local_table,
        environment,
        path_table,
    );
    prepared.rebase_strings(&remap);
    prepared
}

pub(crate) fn finish_prepared_batch(
    prepared: PreparedDiagnosticBatch,
    string_table: StringTable,
) -> CompilationResult<()> {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    accumulator.append_prepared(prepared, None);
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}
