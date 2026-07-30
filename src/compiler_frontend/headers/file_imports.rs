//! Per-file import clause recording for header parsing.
//!
//! WHAT: parses top-level import clauses into normalized file-local import records.
//! WHY: import shells and their local names must be known before declaration headers are built,
//! but full visibility and public-surface validation remain later header-stage responsibilities.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::imports::normalize_import_dependency_path;
use crate::compiler_frontend::headers::types::{FileImport, HeaderExportMode, HeaderParseContext};
use crate::compiler_frontend::paths::const_paths::{
    StructuralProviderReference, parse_import_clause_items,
};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};

/// Boxed diagnostic result for file-local import-clause parsing.
///
/// WHAT: gives the four parsing and recording entry points one small error boundary.
/// WHY: import clauses propagate structured diagnostics through several successful
///      normalization steps without carrying the large value inline at every return.
type FileImportResult<T> = Result<T, Box<CompilerDiagnostic>>;

struct ImportItemRecord {
    provider: StructuralProviderReference,
    authored_provider: StructuralProviderReference,
    alias: Option<StringId>,
    location: SourceLocation,
    alias_location: Option<SourceLocation>,
    from_grouped: bool,
    export_mode: HeaderExportMode,
}

/// Parse and record imports from an explicit `import` clause.
///
/// WHAT: handles ordinary `import @path` by delegating to the shared clause parser with
/// `export_mode: Private`.
pub(super) fn parse_and_record_imports(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    import_location: SourceLocation,
) -> FileImportResult<()> {
    parse_and_record_import_clause(
        token_stream,
        state,
        context,
        HeaderExportMode::Private,
        import_location,
        token_stream.index.saturating_sub(1),
        false,
    )
}

/// Parse a grouped import inside the single public `export:` block.
///
/// WHAT: records grouped source or external imports as public API entries.
/// WHY: the block owns visibility, so a bare namespace import cannot silently become a public
/// export merely because it appears between the block delimiters.
pub(super) fn parse_and_record_public_block_imports(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    import_location: SourceLocation,
) -> FileImportResult<()> {
    parse_and_record_import_clause(
        token_stream,
        state,
        context,
        HeaderExportMode::Public,
        import_location,
        token_stream.index.saturating_sub(1),
        true,
    )
}

fn parse_and_record_import_clause(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    export_mode: HeaderExportMode,
    clause_location: SourceLocation,
    clause_token_index: usize,
    require_grouped: bool,
) -> FileImportResult<()> {
    let (items, next_index) = parse_import_clause_items(
        &token_stream.tokens,
        clause_token_index,
        context.string_table,
    )?;

    if require_grouped && items.iter().any(|item| !item.from_grouped) {
        return Err(Box::new(CompilerDiagnostic::invalid_export_target(
            clause_location,
        )));
    }

    for item in items {
        let mut authored_provider = item.provider.clone();
        authored_provider.from_grouped = item.from_grouped;
        let normalized_path = normalize_import_dependency_path(
            &item.provider.path,
            &token_stream.src_path,
            &item.provider.path_location,
            context.string_table,
        )?;

        record_import_item(
            state,
            ImportItemRecord {
                provider: StructuralProviderReference {
                    path: normalized_path,
                    path_location: item.provider.path_location,
                    from_grouped: item.from_grouped,
                },
                authored_provider,
                alias: item.alias,
                location: clause_location.clone(),
                alias_location: item.alias_location,
                from_grouped: item.from_grouped,
                export_mode,
            },
        );
    }

    token_stream.index = next_index;

    Ok(())
}

/// Record one parsed import item, normalizing duplicate (path, alias) pairs into a single record.
///
/// WHAT: same path + same alias + any public occurrence results in one module public-surface
/// import record.
/// WHY: a module root may repeat an import as a re-export, or import and re-export the same symbol
/// under the same local name. Normalization avoids duplicate records while preserving visibility.
fn record_import_item(state: &mut HeaderFileParseState, record: ImportItemRecord) {
    let local_name = record.alias.or_else(|| record.provider.path.name());
    if let Some(name) = local_name {
        state
            .encountered_symbols
            .insert(name, record.location.clone());
    }

    let key = (record.provider.path.to_owned(), record.alias);

    if state.seen_imports.insert(key.clone()) {
        state
            .file_import_paths
            .insert(record.provider.path.to_owned());
        state.file_imports.push(FileImport {
            provider: record.provider,
            authored_provider: record.authored_provider,
            alias: record.alias,
            location: record.location,
            alias_location: record.alias_location,
            from_grouped: record.from_grouped,
            export_mode: record.export_mode,
        });
    } else {
        // Normalization: if any occurrence is public, upgrade the existing record to public.
        if record.export_mode.is_public() {
            for import in &mut state.file_imports {
                if import.provider.path == key.0 && import.alias == key.1 {
                    import.export_mode = HeaderExportMode::Public;
                    break;
                }
            }
        }
    }
}
