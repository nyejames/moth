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
    ScannedDependencyClause, parse_dependency_clause,
};
use crate::compiler_frontend::headers::dependency_paths::validate_dependency_path;
use crate::compiler_frontend::headers::dependency_target::{
    DependencyTargetKind, classify_dependency_target,
};
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::types::{
    DependencyBindingSyntax, DependencySelection, DependencySelectionRange, HeaderExportMode,
    HeaderParseContext, HeaderParseFailure, RetainedDependencyClause,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};

type FileDependencyClauseResult<T> = Result<T, HeaderParseFailure>;

/// Parse and record an ordinary private dependency clause.
pub(super) fn parse_and_record_private_dependency(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    clause_location: SourceLocation,
) -> FileDependencyClauseResult<()> {
    if context.is_config_file {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            crate::compiler_frontend::compiler_messages::DependencyClauseKind::Namespace,
            crate::compiler_frontend::compiler_messages::InvalidDependencyClauseReason::DependencyClauseNotAllowed,
            clause_location,
        )
        .into());
    }
    parse_and_record_dependency_clause(
        token_stream,
        state,
        context,
        HeaderExportMode::Private,
        clause_location,
        token_stream.index.saturating_sub(1),
        false,
    )
}

/// Parse and record one public dependency clause inside the single `export:` block.
pub(super) fn parse_and_record_public_dependency(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    clause_location: SourceLocation,
) -> FileDependencyClauseResult<()> {
    parse_and_record_dependency_clause(
        token_stream,
        state,
        context,
        HeaderExportMode::Public,
        clause_location,
        token_stream.index.saturating_sub(1),
        true,
    )
}

fn parse_and_record_dependency_clause(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    export_mode: HeaderExportMode,
    clause_location: SourceLocation,
    clause_token_index: usize,
    require_selection_clause: bool,
) -> FileDependencyClauseResult<()> {
    let (parsed, next_index) = parse_dependency_clause(
        &token_stream.tokens,
        clause_token_index,
        &token_stream.path_syntax,
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
    // obsolete provider spelling such as `@./drawing.js` receives the same path diagnostic
    // whether or not the clause also omitted its required binding.
    validate_dependency_path(
        &parsed.provider.path,
        &parsed.provider.path_location,
        context.string_table,
    )?;

    let target = classify_dependency_target(&parsed.provider.path, context.string_table);
    if matches!(
        &parsed.binding,
        ScannedDependencyBinding::Namespace { alias: None }
    ) && matches!(target, DependencyTargetKind::ExternalProvider { .. })
    {
        return Err(CompilerDiagnostic::invalid_dependency_clause(
            crate::compiler_frontend::compiler_messages::DependencyClauseKind::Namespace,
            crate::compiler_frontend::compiler_messages::InvalidDependencyClauseReason::ProviderRequiresBinding,
            parsed.provider.path_location.clone(),
        )
        .into());
    }

    if require_selection_clause
        && !matches!(
            &parsed.binding,
            ScannedDependencyBinding::DirectSelections { selections } if !selections.is_empty()
        )
    {
        return Err(CompilerDiagnostic::invalid_export_target(clause_location).into());
    }

    let file_id = token_stream.file_id.ok_or_else(|| {
        CompilerError::compiler_error(
            "header dependency shell cannot be stamped without a retained source file identity",
        )
    })?;

    let clause_shell_id = DependencyShellId::new(file_id, state.dependency_clause_count as u32);
    state.dependency_clause_count += 1;
    add_frontend_counter(FrontendCounter::DependencyClauseCount, 1);
    add_frontend_counter(FrontendCounter::RetainedShellCount, 1);
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
        clause_location,
        export_mode,
        context.string_table,
    );

    token_stream.index = next_index;
    Ok(())
}

/// Convert one scanned clause into the file-owned retained tables.
fn retain_scanned_clause(
    state: &mut HeaderFileParseState,
    clause_shell_id: DependencyShellId,
    scanned: ScannedDependencyClause,
    target: DependencyTargetKind,
    clause_location: SourceLocation,
    export_mode: HeaderExportMode,
    string_table: &mut StringTable,
) {
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
                let location = selection
                    .local_alias
                    .as_ref()
                    .map_or(&selection.source_location, |alias| &alias.location);
                state
                    .encountered_symbols
                    .entry(local_name)
                    .or_insert_with(|| location.clone());
                state.dependency_selections.push(DependencySelection {
                    source_name: selection.source_name,
                    source_location: selection.source_location,
                    local_alias: selection.local_alias,
                });
            }
            DependencyBindingSyntax::DirectSelections {
                range: DependencySelectionRange::new(start, state.dependency_selections.len()),
            }
        }
    };

    let dependency = RetainedDependencyPath {
        dependency_shell_id: clause_shell_id,
        path: scanned.provider.path,
        target,
        location: scanned.provider.path_location,
    };

    let retained_clause = RetainedDependencyClause {
        dependency,
        binding,
        location: clause_location.clone(),
        export_mode,
    };

    if let Some(name) = retained_clause.effective_namespace_local_name(string_table)
        && let Some(location) = retained_clause.namespace_binding_location()
    {
        state
            .encountered_symbols
            .entry(name)
            .or_insert_with(|| location.clone());
    }

    state.file_dependency_clauses.push(retained_clause);
}
