use super::*;
use crate::compiler_frontend::headers::SourceTokenOwner;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
/// Prepare one owned compiler-semantic source row directly into the module's input lane.
///
/// WHAT: borrows a cached or newly selected snapshot and tokenizes selected Moth sources exactly
///       once, producing the owned input consumed by this module's header preparation queue. The
///       caller's queued set is the ownership proof: a canonical source row is handed to this
///       function at most once for its owning module.
/// WHY: `SourceTreeIndex` owns source identity, while `SourceDatabase` and the selected-text map
///      own source snapshots. Borrowing either avoids a second full source-string allocation while
///      allowing registration-only slots to stay pending until selection.
#[allow(
    clippy::too_many_arguments,
    reason = "owned-source preparation keeps the record index, identity and snapshot sources, span builders, directives, and mutable string/path/selection state as separate borrows"
)]
pub(crate) fn prepare_owned_source_input(
    source_index: SourceRecordIndex,
    source_tree_index: &SourceTreeIndex,
    source_files: &SourceDatabase,
    source_spans: &mut SourceSpanBuilders<'_>,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    selected_source_texts: &mut SelectedSourceTextMap,
) -> Result<PreparedSourceInput, SourceDiscoveryError> {
    let (source_id, kind, text) = owned_source_text(
        source_index,
        source_tree_index,
        source_files,
        Some(selected_source_texts),
    )?;
    let mut builder = source_spans.take_span_builder(source_id);
    let result = prepare_owned_source_text(
        source_id,
        kind,
        text,
        source_files,
        style_directives,
        string_table,
        path_fork,
        &mut builder,
    );
    source_spans.retain_span_builder(source_id, builder);
    result
}
/// Loading failures precede span ownership; a loaded source has exactly one live builder.
fn owned_source_text<'a>(
    source_index: SourceRecordIndex,
    source_tree_index: &SourceTreeIndex,
    source_files: &'a SourceDatabase,
    selected_source_texts: Option<&'a mut SelectedSourceTextMap>,
) -> Result<(SourceId, SourceFileKind, &'a str), SourceDiscoveryError> {
    let record = source_tree_index.source(source_index);
    let SourceClassification::CompilerSemantic(source_kind) = record.classification() else {
        return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
            format!(
                "Project source row {} is not compiler semantic",
                source_index.index()
            ),
        )));
    };
    let source_id = source_files
        .get_by_canonical_path(record.canonical_path())
        .map(|identity| identity.id)
        .ok_or_else(|| {
            SourceDiscoveryError::from(CompilerError::compiler_error(format!(
                "Project source row {} has no registered source identity",
                source_index.index(),
            )))
        })?;
    if let Some(source_code) = source_files.retained_text(source_id) {
        return Ok((source_id, *source_kind, source_code));
    }
    if let Some(error) = source_files.source_load_error(source_id) {
        return Err(SourceDiscoveryError::from(error.clone()));
    }
    let Some(selected_source_texts) = selected_source_texts else {
        return Err(SourceDiscoveryError::from(CompilerError::compiler_error(
            format!(
                "Registered source row {} has no retained source text",
                source_index.index()
            ),
        )));
    };
    let source_code = selected_source_texts
        .load(record.canonical_path())
        .map_err(SourceDiscoveryError::from)?;
    Ok((source_id, *source_kind, source_code))
}
#[allow(
    clippy::too_many_arguments,
    reason = "owned-source tokenization keeps source identity, kind, text, the snapshot database, directives, and mutable string/path/span state as separate borrows"
)]
fn prepare_owned_source_text(
    source_id: SourceId,
    kind: SourceFileKind,
    text: &str,
    source_files: &SourceDatabase,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    builder: &mut ExtendedSpanBuilder,
) -> Result<PreparedSourceInput, SourceDiscoveryError> {
    let source = match kind {
        SourceFileKind::Moth => {
            let identity = source_files
                .get(source_id)
                .expect("owned source identity must be registered");
            let tokenized = tokenize(
                text,
                identity.logical_path,
                TokenizerEntryMode::SourceFile,
                style_directives,
                string_table,
                path_fork,
                source_id,
                builder,
            )
            .map_err(|failure| match failure {
                TokenizeFailure::Diagnosed(diagnostic) => {
                    SourceDiscoveryError::Diagnostic(diagnostic)
                }
                TokenizeFailure::Infrastructure(error) => {
                    SourceDiscoveryError::Infrastructure(error)
                }
            })?;
            if tokenized.logical_path != identity.logical_path || tokenized.file_id != source_id {
                return Err(SourceDiscoveryError::Infrastructure(
                    CompilerError::compiler_error(
                        "lexer source identity does not match its registered owned-source identity",
                    ),
                ));
            }
            let owner = SourceTokenOwner::new(tokenized.tokens);
            PreparedSourceKind::Moth {
                owner,
                path_syntax: tokenized.path_syntax,
            }
        }
        SourceFileKind::MothTemplate | SourceFileKind::PlainMarkdown => {
            PreparedSourceKind::Deferred
        }
    };
    Ok(PreparedSourceInput { source_id, source })
}
