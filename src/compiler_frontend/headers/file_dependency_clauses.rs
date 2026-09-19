//! Per-file dependency clause recording for header parsing.
//!
//! WHAT: parses top-level dependency clauses into one retained provider root and its direct
//!       selections.
//! WHY: one authored clause owns one dependency shell. Stage 0 and later header stages must
//!      consume that ownership instead of receiving one provider row per selected name.
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;

use crate::compiler_frontend::headers::dependency_clause_syntax::{
    DependencyClauseParseError, RetainedDependencyPath, ScannedDependencyBinding,
    ScannedDependencyClause, parse_dependency_clause_at_source,
};
use crate::compiler_frontend::headers::dependency_paths::validate_dependency_path;
use crate::compiler_frontend::headers::dependency_target::{
    DependencyTargetKind, checked_provider_target, classify_dependency_target,
};
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::types::{
    DependencyBindingSyntax, DependencySelection, DependencySelectionRange, HeaderExportMode,
    HeaderParseContext, HeaderParseFailure, RetainedDependencyClause,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TokenCursor, TokenIndex};

type FileDependencyClauseResult<T> = Result<T, HeaderParseFailure>;

fn dependency_clause_token_index(
    cursor: &TokenCursor<'_>,
) -> FileDependencyClauseResult<TokenIndex> {
    let index = cursor.position().index().checked_sub(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "dependency clause token preceded the source token index",
        ))
    })?;
    TokenIndex::try_from_index(index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "dependency clause token exceeded its checked index domain",
        ))
    })
}

pub(super) fn parse_and_record_private_dependency(
    cursor: &mut TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    clause_span: SourceSpan,
) -> FileDependencyClauseResult<()> {
    if context.is_config_file {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            crate::compiler_frontend::compiler_messages::DependencyClauseKind::Namespace,
            crate::compiler_frontend::compiler_messages::InvalidDependencyClauseReason::DependencyClauseNotAllowed,
            Some(clause_span),
        )
        .into());
    }
    parse_and_record_dependency_clause(
        cursor,
        file_id,
        state,
        context,
        HeaderExportMode::Private,
        clause_span,
        dependency_clause_token_index(cursor)?,
        false,
    )
}

pub(super) fn parse_and_record_public_dependency(
    cursor: &mut TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    clause_span: SourceSpan,
) -> FileDependencyClauseResult<()> {
    parse_and_record_dependency_clause(
        cursor,
        file_id,
        state,
        context,
        HeaderExportMode::Public,
        clause_span,
        dependency_clause_token_index(cursor)?,
        true,
    )
}

fn parse_and_record_dependency_clause(
    cursor: &mut TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    export_mode: HeaderExportMode,
    clause_span: SourceSpan,
    clause_token_index: TokenIndex,
    require_selection_clause: bool,
) -> FileDependencyClauseResult<()> {
    let source_tokens = cursor.source_tokens();
    if source_tokens.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "dependency clause source token owner does not match its file identity",
        )));
    }
    let path_syntax = source_tokens
        .path_syntax_table()
        .map_err(HeaderParseFailure::Infrastructure)?;
    let (parsed, next_index) = parse_dependency_clause_at_source(
        source_tokens,
        clause_token_index.index(),
        path_syntax,
        file_id,
    )
    .map_err(|scanner_error| match scanner_error {
        DependencyClauseParseError::Diagnostic(diagnostic) => {
            HeaderParseFailure::Diagnostic(diagnostic)
        }
        DependencyClauseParseError::Infrastructure(error) => {
            HeaderParseFailure::Infrastructure(error)
        }
    })?;

    // Path validity is independent of the selected binding shape. Validate it first so an
    // obsolete provider spelling such as `@./drawing.js` receives the same path diagnostic.
    validate_dependency_path(
        parsed.provider.path,
        &parsed.provider.path_span,
        context.path_fork,
        context.string_table,
    )?;

    let target = classify_dependency_target(
        parsed.provider.path,
        context.path_fork,
        context.string_table,
    );
    if matches!(
        &parsed.binding,
        ScannedDependencyBinding::Namespace { alias: None }
    ) && matches!(target, DependencyTargetKind::ExternalProvider { .. })
    {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            crate::compiler_frontend::compiler_messages::DependencyClauseKind::Namespace,
            crate::compiler_frontend::compiler_messages::InvalidDependencyClauseReason::ProviderRequiresBinding,
            Some(parsed.provider.path_span),
        )
        .into());
    }

    if require_selection_clause
        && !matches!(
            &parsed.binding,
            ScannedDependencyBinding::DirectSelections { selections } if !selections.is_empty()
        )
    {
        return Err(CompilerDiagnostic::invalid_export_target(Some(clause_span)).into());
    }

    let ordinal = u32::try_from(state.dependency_clause_count).map_err(|_| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "dependency clause ordinal exceeded its checked identity domain",
        ))
    })?;
    let clause_shell_id = DependencyShellId::new(file_id, ordinal);
    state.dependency_clause_count =
        state
            .dependency_clause_count
            .checked_add(1)
            .ok_or_else(|| {
                HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                    "dependency clause count overflowed its source state",
                ))
            })?;
    add_frontend_counter(FrontendCounter::DependencyClauseCount, 1);
    let selection_count = match &parsed.binding {
        ScannedDependencyBinding::Namespace { .. } => 0,
        ScannedDependencyBinding::DirectSelections { selections } => selections.len(),
    };
    add_frontend_counter(FrontendCounter::DependencySelectionCount, selection_count);

    retain_scanned_clause(
        state,
        clause_shell_id,
        parsed,
        target,
        export_mode,
        context.string_table,
        &*context.path_fork,
    )?;

    let next_position = TokenIndex::try_from_index(next_index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "dependency clause follower exceeded its checked index domain",
        ))
    })?;
    cursor
        .set_position(next_position)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "dependency clause follower exceeded its source token cursor: {error:?}",
            )))
        })?;
    Ok(())
}

/// Convert one scanned clause into the file-owned retained tables, including its checked provider
/// structure while the worker-local symbol stores are still available.
fn retain_scanned_clause(
    state: &mut HeaderFileParseState,
    clause_shell_id: DependencyShellId,
    scanned: ScannedDependencyClause,
    target: DependencyTargetKind,
    export_mode: HeaderExportMode,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), HeaderParseFailure> {
    let binding = match scanned.binding {
        ScannedDependencyBinding::Namespace { alias } => {
            DependencyBindingSyntax::Namespace { alias }
        }
        ScannedDependencyBinding::DirectSelections { selections } => {
            let start = state.dependency_selections.len();
            for selection in selections {
                let local_name = selection
                    .local_alias
                    .as_ref()
                    .map_or(selection.source_name, |alias| alias.name);
                let span = selection
                    .local_alias
                    .as_ref()
                    .map_or(selection.source_span, |alias| alias.span);
                state.encountered_symbols.entry(local_name).or_insert(span);
                state.dependency_selections.push(DependencySelection {
                    source_name: selection.source_name,
                    source_span: selection.source_span,
                    local_alias: selection.local_alias,
                });
            }
            DependencyBindingSyntax::DirectSelections {
                range: DependencySelectionRange::new(start, state.dependency_selections.len()),
            }
        }
    };

    let provider_target =
        checked_provider_target(scanned.provider.path, &target, path_fork, string_table)?;
    let dependency = RetainedDependencyPath {
        dependency_shell_id: clause_shell_id,
        path: scanned.provider.path,
        path_syntax: scanned.provider.path_syntax,
        target,
        provider_target,
        local_source_id: None,
        span: scanned.provider.path_span,
    };

    let retained_clause = RetainedDependencyClause {
        dependency,
        binding,
        export_mode,
    };
    if let Some(name) = retained_clause.effective_namespace_local_name(string_table, path_fork)
        && let Some(span) = retained_clause.namespace_binding_span()
    {
        state.encountered_symbols.entry(name).or_insert(*span);
    }

    state.file_dependency_clauses.push(retained_clause);
    Ok(())
}
