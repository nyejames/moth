//! Per-file header splitting.
//!
//! WHAT: orchestrates one tokenized Moth file into top-level declaration headers, dependency
//! records, const-fragment metadata, and the implicit entry `start` body.
//! WHY: file-level control flow is different from declaration parsing, dependency recording, and hash
//! item handling; this module keeps the high-level loop visible while delegated modules own details.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, InvalidConfigReason, InvalidDeclarationReason,
    InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::declaration_syntax::build_config_contract::find_config_qualifier_marker_in_cursor;
use crate::compiler_frontend::headers::file_dependency_clauses::{
    parse_and_record_private_dependency, parse_and_record_public_dependency,
};
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::hash_items::handle_hash_item;
use crate::compiler_frontend::headers::header_dispatch::create_header;
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::parse_file_headers::find_config_qualifier_marker_in_declaration_defaults;
use crate::compiler_frontend::headers::start_capture::capture_runtime_template_range_from_cursor;
use crate::compiler_frontend::headers::symbol_collection::is_receiver_method_candidate;
use crate::compiler_frontend::headers::top_level_classifier::{
    HeaderFileItem, classify_current_item_ref, classify_export_block_item_ref,
    starts_duplicate_top_level_header_declaration_at_source,
    starts_specialized_generic_conformance_declaration_at_source,
    starts_trait_declaration_after_must_at_source, statement_boundary_at_source,
};
use crate::compiler_frontend::headers::types::{
    DependencySelection, FileFrontendPrepareFailure, FileFrontendPrepareOutput, FileRole, Header,
    HeaderBuildContext, HeaderExportMode, HeaderKind, HeaderParseContext, HeaderParseFailure,
    RetainedDependencyClause,
};
use crate::compiler_frontend::paths::const_paths::can_serialize_path_component_bare;
use crate::compiler_frontend::paths::file_references::classify_prepared_file_references;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::source_packages::root_file::file_name_is_config_file;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
};
use crate::compiler_frontend::utilities::token_scan::{ScannedToken, TokenFactView};
use rustc_hash::FxHashSet;

type FileParserResult<T> = Result<T, HeaderParseFailure>;

fn diagnostic_failure(diagnostic: CompilerDiagnostic) -> HeaderParseFailure {
    HeaderParseFailure::Diagnostic(diagnostic)
}
/// Resolve one checked source-owned token at a compatibility cursor index.
///
/// The compatibility vector remains available to deferred parser callees, but all Stage 0 facts
/// come from the canonical source owner.
fn source_token_at_index<'a>(
    canonical: &'a SourceTokens,
    file_id: SourceId,
    index: TokenIndex,
    owner: &str,
) -> FileParserResult<TokenRef<'a>> {
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(format!(
                "{owner} source token owner does not match its file identity"
            )),
        ));
    }
    canonical.token(index).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "{owner} token index exceeded its source token owner: {error:?}"
        )))
    })
}

fn record_start_body_token_from_source(
    state: &mut HeaderFileParseState,
    canonical: &SourceTokens,
    file_id: SourceId,
    index: TokenIndex,
) -> FileParserResult<()> {
    let token = source_token_at_index(canonical, file_id, index, "start-body token")?;
    state
        .record_start_body_token_ref(token)
        .map_err(HeaderParseFailure::Infrastructure)
}

// Top-level declarations are same-module-visible by default; cross-module public visibility
// comes only from the root `export:` block. Non-declaration statements are collected into the
// implicit start-function header for that file.
pub(super) fn parse_headers_in_file(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    token_count: usize,
    context: &mut HeaderParseContext<'_>,
) -> Result<HeaderFileParseState, FileFrontendPrepareFailure> {
    let mut state = HeaderFileParseState::new(token_count);
    match parse_headers_in_file_inner(cursor, file_id, source_file, context, &mut state) {
        Ok(()) => Ok(state),
        Err(HeaderParseFailure::Diagnostic(diagnostic)) => {
            Err(FileFrontendPrepareFailure::Diagnosed(state.into_error(diagnostic)))
        }
        Err(HeaderParseFailure::Infrastructure(error)) => {
            Err(FileFrontendPrepareFailure::Infrastructure(error))
        }
    }
}

fn parse_headers_in_file_inner(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    context: &mut HeaderParseContext<'_>,
    state: &mut HeaderFileParseState,
) -> FileParserResult<()> {
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(
                "header walk source token owner does not match its file identity",
            ),
        ));
    }

    loop {
        let current_index = cursor.position();
        let current = cursor.current().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "header cursor exceeded its source token owner",
            ))
        })?;
        let follower_index = current_index.index().checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "header token follower index overflowed its source range",
            ))
        })?;
        let follower = TokenIndex::try_from_index(follower_index)
            .and_then(|index| canonical.token(index).ok());
        let at_boundary = statement_boundary_at_source(canonical, current_index.index());
        let current_span = current.source_span();
        let current_tag = current.tag();
        let item = classify_current_item_ref(current, follower, at_boundary);
        cursor.advance();
        match item {
            HeaderFileItem::Symbol(name_id) => {
                handle_symbol_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    name_id,
                    current_span,
                    current_index,
                )?;
            }

            HeaderFileItem::BuiltinTypeConformanceTarget(type_name) => {
                let name_id = context.string_table.intern(type_name);
                handle_symbol_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    name_id,
                    current_span,
                    current_index,
                )?;
            }

            HeaderFileItem::Dependency => {
                parse_and_record_private_dependency(cursor, file_id, state, context, current_span)?;
            }

            HeaderFileItem::Export => {
                reject_non_block_export(cursor, file_id, context, current_span)?;
            }

            HeaderFileItem::ExportBlock => {
                handle_export_block(cursor, file_id, source_file, state, context, current_span)?;
            }

            HeaderFileItem::Hash {
                at_statement_boundary,
            } => {
                handle_hash_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    current_index,
                    current_span,
                    at_statement_boundary,
                )?;
            }

            HeaderFileItem::ReservedTraitSyntax => {
                handle_trait_keyword_header_item(current_tag, current_span)?;
            }

            HeaderFileItem::RuntimeTemplate => {
                handle_runtime_template_item(cursor, file_id, state, context)?;
            }

            HeaderFileItem::Eof => {
                record_start_body_token_from_source(state, canonical, file_id, current_index)?;
                break;
            }

            HeaderFileItem::StartBodyToken => {
                record_start_body_token_from_source(state, canonical, file_id, current_index)?;
            }
        }
    }

    Ok(())
}

fn reject_non_block_export(
    cursor: &TokenCursor<'_>,
    file_id: SourceId,
    context: &mut HeaderParseContext<'_>,
    export_span: SourceSpan,
) -> FileParserResult<()> {
    // `export` is valid only as the module-root `export:` block.
    if !context.file_role.is_export_capable() || context.is_config_file {
        return Err(diagnostic_failure(
            CompilerDiagnostic::export_outside_module_root(Some(export_span)),
        ));
    }

    // Without the block delimiter, the token is not an export target. Keep this diagnostic in
    // header parsing instead of interpreting the following tokens through another syntax path.
    let found = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "export-block cursor exceeded its source token owner",
        ))
    })?;
    if found.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "export-block source token owner does not match its file identity",
        )));
    }
    Err(diagnostic_failure(
        CompilerDiagnostic::expected_token_from_ref(
            TokenTag::COLON,
            Some(found),
            Some(export_span),
        ),
    ))
}
fn handle_export_block(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    export_span: SourceSpan,
) -> FileParserResult<()> {
    if !context.file_role.is_export_capable() || context.is_config_file {
        return Err(diagnostic_failure(
            CompilerDiagnostic::export_outside_module_root(Some(export_span)),
        ));
    }

    if state.seen_export_block.is_some() {
        return Err(diagnostic_failure(
            CompilerDiagnostic::duplicate_export_block(Some(export_span)),
        ));
    }

    // The classifier only produces ExportBlock when the current token is `:`, but consume it
    // here so the item parser starts at the first ordinary top-level item.
    let colon = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "export-block cursor exceeded its source token owner",
        ))
    })?;
    if colon.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "export-block source token owner does not match its file identity",
        )));
    }
    if colon.tag() != TokenTag::COLON {
        return Err(diagnostic_failure(
            CompilerDiagnostic::expected_token_from_ref(
                TokenTag::COLON,
                Some(colon),
                Some(export_span),
            ),
        ));
    }
    state.seen_export_block = Some(export_span);
    state.export_mode = HeaderExportMode::Public;
    cursor.advance();

    loop {
        let current = cursor.current().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "export-block cursor exceeded its source token owner",
            ))
        })?;
        let current_tag = current.tag();
        if current_tag == TokenTag::END || current_tag == TokenTag::EOF {
            break;
        }
        if current_tag == TokenTag::NEWLINE {
            cursor.advance();
            continue;
        }

        let current_index = current.index();
        let follower_index = current_index.index().checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "export-block follower index overflowed its source range",
            ))
        })?;
        let follower = TokenIndex::try_from_index(follower_index)
            .and_then(|index| cursor.source_tokens().token(index).ok());
        let current_span = current.source_span();
        let current_tag = current.tag();
        let item = classify_export_block_item_ref(current, follower);
        cursor.advance();
        parse_export_block_item(
            cursor,
            file_id,
            source_file,
            state,
            context,
            item,
            current_tag,
            current_span,
            current_index,
        )?;
        state.export_block_item_count =
            state
                .export_block_item_count
                .checked_add(1)
                .ok_or_else(|| {
                    HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                        "export-block item count overflowed its source state",
                    ))
                })?;
    }

    let current = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "export-block cursor exceeded its source token owner",
        ))
    })?;
    if current.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "export-block source token owner does not match its file identity",
        )));
    }
    if current.tag() == TokenTag::EOF {
        return Err(diagnostic_failure(
            CompilerDiagnostic::unexpected_end_of_file(
                Some(context.string_table.intern(";")),
                Some(current.source_span()),
            ),
        ));
    }

    // The block terminator belongs to this parser mode and must not become an implicit start-body
    // token for the surrounding file.
    cursor.advance();
    state.export_mode = HeaderExportMode::Private;

    if state.export_block_item_count == 0 {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(Some(export_span)),
        ));
    }

    Ok(())
}

fn parse_export_block_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    item: HeaderFileItem,
    current_tag: TokenTag,
    current_span: SourceSpan,
    current_index: TokenIndex,
) -> FileParserResult<()> {
    match item {
        HeaderFileItem::Symbol(name_id) => handle_symbol_item(
            cursor,
            file_id,
            source_file,
            state,
            context,
            name_id,
            current_span,
            current_index,
        ),

        HeaderFileItem::BuiltinTypeConformanceTarget(type_name) => {
            let name_id = context.string_table.intern(type_name);
            handle_symbol_item(
                cursor,
                file_id,
                source_file,
                state,
                context,
                name_id,
                current_span,
                current_index,
            )
        }

        HeaderFileItem::Dependency => {
            parse_and_record_public_dependency(cursor, file_id, state, context, current_span)
        }

        HeaderFileItem::Export | HeaderFileItem::ExportBlock => Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(Some(current_span)),
        )),

        HeaderFileItem::Hash {
            at_statement_boundary,
        } => handle_hash_item(
            cursor,
            file_id,
            source_file,
            state,
            context,
            current_index,
            current_span,
            at_statement_boundary,
        ),

        HeaderFileItem::RuntimeTemplate | HeaderFileItem::StartBodyToken => {
            Err(diagnostic_failure(
                CompilerDiagnostic::invalid_export_target(Some(current_span)),
            ))
        }

        HeaderFileItem::ReservedTraitSyntax => {
            if let Some(keyword) = reserved_trait_keyword_for_tag(current_tag) {
                return Err(diagnostic_failure(reserved_trait_keyword_error(
                    keyword,
                    Some(current_span),
                )));
            }

            Err(diagnostic_failure(
                CompilerDiagnostic::invalid_export_target(Some(current_span)),
            ))
        }

        HeaderFileItem::Eof => Err(diagnostic_failure(
            CompilerDiagnostic::unexpected_end_of_file(
                Some(context.string_table.intern(";")),
                Some(current_span),
            ),
        )),
    }
}

/// The recognised start of a removed `import` `@path` clause.
///
/// WHAT: records the `import` token and the following path token after optional newlines.
/// WHY: end scanning, replacement generation and diagnostic spans must share one path index
///      instead of independently skipping trivia from `import` again.
struct LegacyDependencyStart {
    import_index: TokenIndex,
    path_index: TokenIndex,
}

fn checked_legacy_index(index: usize, offset: usize, owner: &str) -> FileParserResult<usize> {
    index.checked_add(offset).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "{owner} token index overflowed its source range"
        )))
    })
}

/// Recognise only `import`, optional newlines, then a path token.
///
/// Comments are not produced as tokens, so comments between the keyword and path are
/// already transparent. `import = 1` and `import` followed by an unrelated statement
/// do not match.
fn recognize_legacy_dependency_start(
    tokens: TokenFactView<'_>,
    import_index: TokenIndex,
    string_table: &StringTable,
) -> FileParserResult<Option<LegacyDependencyStart>> {
    let Some(import) = tokens.get(import_index.index()) else {
        return Ok(None);
    };
    if import.tag != TokenTag::SYMBOL
        || import
            .symbol
            .is_none_or(|name| string_table.resolve(name) != "import")
    {
        return Ok(None);
    }

    let mut index = checked_legacy_index(import_index.index(), 1, "legacy dependency")?;
    while tokens
        .get(index)
        .is_some_and(|token| token.tag == TokenTag::NEWLINE)
    {
        index = checked_legacy_index(index, 1, "legacy dependency")?;
    }

    if !tokens
        .get(index)
        .is_some_and(|token| token.tag == TokenTag::PATH)
    {
        return Ok(None);
    }
    let path_index = TokenIndex::try_from_index(index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "legacy dependency path index exceeded its checked domain",
        ))
    })?;
    Ok(Some(LegacyDependencyStart {
        import_index,
        path_index,
    }))
}

fn handle_symbol_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    name_id: StringId,
    current_span: SourceSpan,
    current_index: TokenIndex,
) -> FileParserResult<()> {
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(
                "legacy dependency scan source token owner does not match its file identity",
            ),
        ));
    }
    let facts = TokenFactView::from_source(canonical);
    if let Some(start) = recognize_legacy_dependency_start(
        facts,
        current_index,
        context.string_table,
    )? {
        let path_syntax = (!context.is_config_file)
            .then(|| canonical.path_syntax_table())
            .transpose()
            .map_err(HeaderParseFailure::Infrastructure)?;
        return Err(diagnostic_failure(legacy_dependency_clause_diagnostic(
            facts,
            file_id,
            path_syntax,
            context,
            current_span,
            start,
        )?));
    }

    let export_mode = state.export_mode;
    handle_symbol_item_with_export_mode(
        cursor,
        file_id,
        source_file,
        state,
        context,
        SymbolItemRequest {
            name_id,
            current_span,
            current_index,
        },
        export_mode,
    )
}

/// Build the one-way migration diagnostic for the removed `import @...` grammar.
///
/// A replacement is offered only when the old clause maps mechanically to one current clause.
/// Filtered namespaces and nested groups deliberately require an author choice.
fn legacy_dependency_clause_diagnostic(
    tokens: TokenFactView<'_>,
    source_id: SourceId,
    path_syntax: Option<&PathSyntaxTable>,
    context: &mut HeaderParseContext<'_>,
    current_span: SourceSpan,
    start: LegacyDependencyStart,
) -> Result<CompilerDiagnostic, HeaderParseFailure> {
    let start_span = tokens
        .get(start.import_index.index())
        .map(|token| SourceSpan::new(source_id, token.span))
        .unwrap_or(current_span);
    let end_span = legacy_dependency_clause_end(tokens, start.path_index)?
        .and_then(|index| tokens.get(index.index()))
        .map(|token| SourceSpan::new(source_id, token.span))
        .unwrap_or(start_span);
    let clause_span = start_span
        .join(end_span, context.span_builder)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "legacy dependency clause span could not be joined: {error:?}"
            )))
        })?;

    let replacement = if context.is_config_file {
        None
    } else {
        let path_syntax = path_syntax.ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "legacy dependency replacement is missing its path syntax table",
            ))
        })?;
        legacy_dependency_replacement(
            tokens,
            source_id,
            path_syntax,
            context.string_table,
            context.path_fork,
            start.path_index,
        )?
        .map(|replacement| context.string_table.intern(&replacement))
    };
    Ok(CompilerDiagnostic::legacy_dependency_clause(
        replacement,
        Some(clause_span),
    ))
}

fn legacy_dependency_clause_end(
    tokens: TokenFactView<'_>,
    path_index: TokenIndex,
) -> FileParserResult<Option<TokenIndex>> {
    let mut index = path_index.index();
    let mut brace_depth = 0usize;
    let mut last = None;
    while let Some(token) = tokens.get(index) {
        match token.tag {
            TokenTag::OPEN_CURLY => {
                brace_depth = brace_depth.checked_add(1).ok_or_else(|| {
                    HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                        "legacy dependency brace depth overflowed its source range",
                    ))
                })?;
            }
            TokenTag::CLOSE_CURLY => brace_depth = brace_depth.saturating_sub(1),
            TokenTag::NEWLINE if brace_depth == 0 => break,
            TokenTag::END | TokenTag::EOF => break,
            _ => {}
        }
        last = Some(TokenIndex::try_from_index(index).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "legacy dependency clause index exceeded its checked domain",
            ))
        })?);
        index = checked_legacy_index(index, 1, "legacy dependency clause")?;
    }
    Ok(last)
}

fn legacy_dependency_replacement(
    tokens: TokenFactView<'_>,
    source_id: SourceId,
    path_syntax: &PathSyntaxTable,
    string_table: &StringTable,
    path_fork: &crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    path_index: TokenIndex,
) -> Result<Option<String>, HeaderParseFailure> {
    let Some(path_token) = tokens.get(path_index.index()) else {
        return Ok(None);
    };
    if path_token.tag != TokenTag::PATH {
        return Ok(None);
    }
    let Some(path_syntax_id) = path_token.path_syntax_id else {
        return Ok(None);
    };
    let path = path_syntax
        .try_path_for_token(path_syntax_id, SourceSpan::new(source_id, path_token.span))?
        .root;
    if path == PathId::ROOT {
        // Exact `@/` is represented by the empty canonical path, and so is the bare introducer
        // `@`. The retained row cannot tell the two spellings apart, so it suggests neither.
        return Ok(None);
    }
    let mut components = Vec::new();
    path_fork.resolve_components(path, &mut components);
    if components
        .iter()
        .any(|component| !can_serialize_path_component_bare(string_table.resolve(*component)))
    {
        // The canonical path row intentionally does not retain component quoting. Do not emit a
        // normalized spelling that would turn a valid quoted path into invalid unquoted syntax.
        return Ok(None);
    }
    let path = path_fork.render_portable(path, string_table, &mut components);
    let mut replacement = format!("@{path}");
    let mut index = checked_legacy_index(path_index.index(), 1, "legacy dependency replacement")?;

    if tokens
        .get(index)
        .is_some_and(|token| token.tag == TokenTag::AS)
    {
        let alias_index = checked_legacy_index(index, 1, "legacy dependency replacement")?;
        let Some(alias) = tokens.get(alias_index) else {
            return Ok(None);
        };
        let Some(alias_name) = alias.symbol.filter(|_| alias.tag == TokenTag::SYMBOL) else {
            return Ok(None);
        };
        replacement.push_str(" as ");
        replacement.push_str(string_table.resolve(alias_name));
        index = checked_legacy_index(alias_index, 1, "legacy dependency replacement")?;
        return Ok(clause_terminator(tokens.get(index)).then_some(replacement));
    }

    if !tokens
        .get(index)
        .is_some_and(|token| token.tag == TokenTag::OPEN_CURLY)
    {
        return Ok(clause_terminator(tokens.get(index)).then_some(replacement));
    }
    index = checked_legacy_index(index, 1, "legacy dependency replacement")?;
    replacement.push(' ');

    let mut first_selection = true;
    loop {
        while tokens
            .get(index)
            .is_some_and(|token| token.tag == TokenTag::NEWLINE)
        {
            index = checked_legacy_index(index, 1, "legacy dependency replacement")?;
        }
        let Some(token) = tokens.get(index) else {
            return Ok(None);
        };
        if token.tag == TokenTag::CLOSE_CURLY {
            let after_close = checked_legacy_index(index, 1, "legacy dependency replacement")?;
            return Ok(
                (!first_selection && clause_terminator(tokens.get(after_close)))
                    .then_some(replacement),
            );
        }
        let Some(name) = token.symbol.filter(|_| token.tag == TokenTag::SYMBOL) else {
            return Ok(None);
        };
        if !first_selection {
            replacement.push_str(", ");
        }
        replacement.push_str(string_table.resolve(name));
        first_selection = false;
        index = checked_legacy_index(index, 1, "legacy dependency replacement")?;

        if tokens
            .get(index)
            .is_some_and(|token| token.tag == TokenTag::AS)
        {
            let alias_index = checked_legacy_index(index, 1, "legacy dependency replacement")?;
            let Some(alias) = tokens.get(alias_index) else {
                return Ok(None);
            };
            let Some(alias_name) = alias.symbol.filter(|_| alias.tag == TokenTag::SYMBOL) else {
                return Ok(None);
            };
            replacement.push_str(" as ");
            replacement.push_str(string_table.resolve(alias_name));
            index = checked_legacy_index(alias_index, 1, "legacy dependency replacement")?;
        }

        match tokens.get(index).map(|token| token.tag) {
            Some(TokenTag::COMMA) => {
                index = checked_legacy_index(index, 1, "legacy dependency replacement")?
            }
            Some(TokenTag::CLOSE_CURLY | TokenTag::NEWLINE) => {}
            _ => return Ok(None),
        }
    }
}

fn clause_terminator(token: Option<ScannedToken>) -> bool {
    token.is_none_or(|token| matches!(token.tag, TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF))
}

/// One already-read top-level symbol with the source facts its header dispatch needs.
struct SymbolItemRequest {
    name_id: StringId,
    current_span: SourceSpan,
    current_index: TokenIndex,
}

fn handle_symbol_item_with_export_mode(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    symbol: SymbolItemRequest,
    export_mode: HeaderExportMode,
) -> FileParserResult<()> {
    let SymbolItemRequest {
        name_id,
        current_span,
        current_index,
    } = symbol;
    let follower_index = TokenIndex::try_from_index(cursor.position().index()).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "symbol lookahead index exceeded its checked domain",
        ))
    })?;
    let (follower_tag, starts_duplicate, starts_trait, starts_specialized) = {
        let canonical = cursor.source_tokens();
        if canonical.source() != file_id {
            return Err(HeaderParseFailure::Infrastructure(
                CompilerError::compiler_error(
                    "symbol lookahead source token owner does not match its file identity",
                ),
            ));
        }
        let follower = canonical.token(follower_index).map_err(|_| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "symbol lookahead index exceeded its source token owner",
            ))
        })?;
        (
            follower.tag(),
            starts_duplicate_top_level_header_declaration_at_source(canonical, follower_index),
            starts_trait_declaration_after_must_at_source(canonical, follower_index),
            starts_specialized_generic_conformance_declaration_at_source(canonical, follower_index),
        )
    };

    if export_mode.is_public() && !starts_duplicate {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(Some(current_span)),
        ));
    }

    if let Some(first_span) = state.encountered_symbols.get(&name_id) {
        let is_conformance_declaration =
            (follower_tag == TokenTag::MUST && !starts_trait) || starts_specialized;

        if !is_conformance_declaration && starts_duplicate {
            return Err(diagnostic_failure(
                CompilerDiagnostic::duplicate_declaration(
                    name_id,
                    Some(*first_span),
                    Some(current_span),
                ),
            ));
        }

        if !is_conformance_declaration {
            record_start_body_token_from_source(
                state,
                cursor.source_tokens(),
                file_id,
                current_index,
            )?;
            return Ok(());
        }
    }

    if state.start_body_symbols.contains(&name_id) && !starts_duplicate {
        record_start_body_token_from_source(
            state,
            cursor.source_tokens(),
            file_id,
            current_index,
        )?;
        return Ok(());
    }

    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file,
        file_dependency_clauses: &state.file_dependency_clauses,
        dependency_selections: &state.dependency_selections,
        string_table: context.string_table,
        path_fork: context.path_fork,
        file_role: context.file_role,
    };
    let declaration_path = build_context
        .path_fork
        .try_intern_child(source_file, name_id)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "path table exhausted while interning declaration path",
            ))
        })?;
    let header = create_header(
        declaration_path,
        cursor,
        file_id,
        current_span,
        current_index.index(),
        export_mode,
        &mut build_context,
        context.span_builder,
    )?;

    if export_mode.is_public()
        && matches!(
            &header.kind,
            HeaderKind::TraitConformance { .. } | HeaderKind::TraitIncompatibility { .. }
        )
    {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(Some(current_span)),
        ));
    }

    if export_mode.is_public()
        && let HeaderKind::Function { signature, .. } = &header.kind
        && is_receiver_method_candidate(signature, context.string_table, context.path_fork)
    {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
                Some(current_span),
            ),
        ));
    }

    match &header.kind {
        HeaderKind::StartFunction => {
            record_start_body_token_from_source(
                state,
                cursor.source_tokens(),
                file_id,
                current_index,
            )?;
            state.register_start_body_symbol(name_id);
        }
        HeaderKind::TraitConformance { .. } | HeaderKind::TraitIncompatibility { .. } => {
            state.register_header(header);
        }

        _ => {
            let name_span = header
                .name_span
                .expect("authored declaration headers carry a source span");
            state.register_header(header);
            state.encountered_symbols.insert(name_id, name_span);
        }
    }

    Ok(())
}

fn handle_trait_keyword_header_item(
    current_tag: TokenTag,
    current_span: SourceSpan,
) -> FileParserResult<()> {
    if let Some(keyword) = reserved_trait_keyword_for_tag(current_tag) {
        return Err(diagnostic_failure(reserved_trait_keyword_error(
            keyword,
            Some(current_span),
        )));
    }

    Ok(())
}

fn handle_runtime_template_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
) -> FileParserResult<()> {
    // Runtime top-level templates stay in the start-function body and are evaluated in source
    // order by entry start(). The runtime fragment count lets later const fragments record their
    // insertion point relative to already-seen runtime fragments.
    let opening_index = cursor.position().index().checked_sub(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template opening preceded the source token index",
        ))
    })?;
    let opening_index = TokenIndex::try_from_index(opening_index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template opening exceeded the source token index space",
        ))
    })?;
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template source token owner does not match its file identity",
        )));
    }
    let opening = canonical.token(opening_index).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "runtime template opening exceeded its source token owner: {error:?}",
        )))
    })?;
    let range = capture_runtime_template_range_from_cursor(
        opening,
        cursor,
        file_id,
        context.string_table,
    )?;
    state
        .record_start_body_source_range_from_source_tokens(range, canonical)
        .map_err(HeaderParseFailure::Infrastructure)?;
    if context.file_role == FileRole::ActiveModuleRoot {
        state.runtime_fragment_count =
            state.runtime_fragment_count.checked_add(1).ok_or_else(|| {
                HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                    "runtime fragment count overflowed its source state",
                ))
            })?;
    }

    Ok(())
}
fn find_config_marker_in_start_ranges(
    state: &HeaderFileParseState,
    canonical: &SourceTokens,
    file_id: SourceId,
    string_table: &StringTable,
    span_builder: &crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> Result<Option<(SourceSpan, bool)>, CompilerError> {
    if canonical.source() != file_id {
        return Err(CompilerError::compiler_error(
            "start-body config scan source token owner does not match its file identity",
        ));
    }
    for range in &state.start_body_ranges {
        if range.source() != file_id {
            return Err(CompilerError::compiler_error(
                "start-body config scan encountered a foreign source range",
            ));
        }
        let cursor = canonical.cursor(*range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "start-body config scan exceeded source token bounds: {error:?}"
            ))
        })?;
        if let Some(marker) =
            find_config_qualifier_marker_in_cursor(cursor, string_table, span_builder)
        {
            return Ok(Some(marker));
        }
    }
    Ok(None)
}
pub(super) fn finish_file_output(
    token_stream: &mut FileTokens,
    file_id: SourceId,
    end_index: TokenIndex,
    context: &mut HeaderParseContext<'_>,
    state: HeaderFileParseState,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let canonical = token_stream
        .source_tokens()
        .map_err(FileFrontendPrepareFailure::Infrastructure)?;
    if context.file_role == FileRole::ImportedModuleRoot
        && let Some((marker_span, adjacent)) = find_config_marker_in_start_ranges(
            &state,
            canonical,
            file_id,
            context.string_table,
            context.span_builder,
        )
        .map_err(FileFrontendPrepareFailure::Infrastructure)?
    {
        let diagnostic = if adjacent {
            CompilerDiagnostic::invalid_config_reason(
                None,
                InvalidConfigReason::ConfigQualifierInvalidPlacement,
                Some(marker_span),
            )
        } else {
            CompilerDiagnostic::common_syntax_mistake(
                CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
                Some(marker_span),
            )
        };
        return Err(FileFrontendPrepareFailure::Diagnosed(
            state.into_error(diagnostic),
        ));
    }

    if let Some(diagnostic) = dependency_generic_parameter_collision(
        &state.headers,
        &state.file_dependency_clauses,
        &state.dependency_selections,
        context.string_table,
        &*context.path_fork,
    ) {
        return Err(FileFrontendPrepareFailure::Diagnosed(
            state.into_error(diagnostic),
        ));
    }

    if matches!(
        context.file_role,
        FileRole::Normal | FileRole::ActiveApiOnlyModuleRoot
    ) && state.has_non_trivial_start_body()
    {
        let span = state
            .first_executable_start_body_span()
            .expect("non-trivial start body has an executable token span");
        return Err(FileFrontendPrepareFailure::Diagnosed(state.into_error(
            CompilerDiagnostic::invalid_top_level_runtime_statement(Some(span)),
        )));
    }

    let mut output = if context.file_role == FileRole::ActiveModuleRoot {
        state
            .into_entry_output(token_stream, end_index, context.file_role)
            .map_err(FileFrontendPrepareFailure::Infrastructure)?
    } else {
        state
            .into_non_entry_output(token_stream, context.file_role)
            .map_err(FileFrontendPrepareFailure::Infrastructure)?
    };
    // The compatibility stream is only a parser boundary. Publish its existing canonical
    // allocation directly; moving the stream itself would retain a legacy `FileTokens` shell in
    // the prepared-source handoff.
    let canonical_owner = token_stream
        .canonical_source_tokens_arc()
        .map_err(FileFrontendPrepareFailure::Infrastructure)?;
    attach_structural_file_facts(
        &mut output,
        &canonical_owner,
        context.string_table,
        context.path_fork,
    )
    .map_err(FileFrontendPrepareFailure::Infrastructure)?;
    output
        .install_source_token_stream(canonical_owner)
        .map_err(FileFrontendPrepareFailure::Infrastructure)?;
    Ok(output)
}

/// Classify this file's graph-active file-value paths and derive shell ordering facts from them.
///
/// WHAT: classifies every non-dependency path row once, then records content-source ordering
///       hints from the classified rows into each declaration shell that folds before body
///       emission.
/// WHY: classification is the single graph-activity fact source, and the content ordering edges
///       must come from the same prepared rows at token level rather than an expression parse.
///       Source `#Config` initializers are excluded until their primitive-only contract validation
///       succeeds, so an invalid default cannot affect Stage 0 source discovery.
fn attach_structural_file_facts(
    output: &mut FileFrontendPrepareOutput,
    canonical: &SourceTokens,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), CompilerError> {
    let is_config_file = path_fork
        .component(output.source_file)
        .is_some_and(|name| file_name_is_config_file(string_table.resolve(name)));
    if canonical.source() != output.file_id {
        return Err(CompilerError::compiler_error(
            "source token owner does not match its file stream identity",
        ));
    }
    let mut consumed_path_syntax_ids = Vec::new();
    consumed_path_syntax_ids.extend(
        output
            .file_dependency_clauses
            .iter()
            .map(|clause| clause.dependency.path_syntax),
    );
    fn append_source_range_paths(
        output: &mut Vec<crate::compiler_frontend::paths::path_syntax::PathSyntaxId>,
        canonical: &SourceTokens,
        range: Option<TokenRange>,
    ) -> Result<(), CompilerError> {
        let Some(range) = range else {
            return Ok(());
        };
        let cursor = canonical.cursor(range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "retained declaration range could not be resolved for path collection: {error:?}"
            ))
        })?;
        append_source_paths(output, cursor);
        Ok(())
    }
    fn append_source_paths(
        output: &mut Vec<crate::compiler_frontend::paths::path_syntax::PathSyntaxId>,
        mut cursor: TokenCursor<'_>,
    ) {
        while let Some(token) = cursor.advance() {
            if let Some(path_syntax_id) = token.path_syntax_id() {
                output.push(path_syntax_id);
            }
            if token.is_eof() {
                break;
            }
        }
    }

    for header in &output.headers {
        if header.tokens.source() != output.file_id {
            return Err(CompilerError::compiler_error(
                "retained header range does not match its source token owner",
            ));
        }
        let declaration_owned_marker = matches!(
            &header.kind,
            HeaderKind::Constant { declaration } if declaration.config_qualifier.is_some()
        );
        let body_cursor = if let Some(sequence) = header.token_sequence {
            let view = canonical.token_sequence(sequence).map_err(|error| {
                CompilerError::compiler_error(format!(
                    "retained header token sequence could not be resolved: {error:?}"
                ))
            })?;
            Some(view.cursor().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "retained header token sequence cursor could not be constructed: {error:?}"
                ))
            })?)
        } else if header.tokens.is_empty() {
            None
        } else {
            Some(canonical.cursor(header.tokens).map_err(|error| {
                CompilerError::compiler_error(format!(
                    "retained header token range could not be resolved: {error:?}"
                ))
            })?)
        };
        let body_has_config_marker = body_cursor
            .is_some_and(|cursor| source_cursor_contains_config_marker(cursor, string_table));
        let declaration_has_config_marker = if declaration_owned_marker {
            false
        } else {
            find_config_qualifier_marker_in_declaration_defaults(header, canonical, string_table)?
                .is_some()
        };
        if is_config_file
            || (!declaration_owned_marker
                && !body_has_config_marker
                && !declaration_has_config_marker)
        {
            continue;
        }

        if let Some(cursor) = body_cursor {
            append_source_paths(&mut consumed_path_syntax_ids, cursor);
        }
        match &header.kind {
            HeaderKind::Constant { declaration } => {
                append_source_range_paths(
                    &mut consumed_path_syntax_ids,
                    canonical,
                    declaration.initializer_range,
                )?;
            }
            HeaderKind::Function { signature, .. } => {
                for parameter in &signature.parameters {
                    append_source_range_paths(
                        &mut consumed_path_syntax_ids,
                        canonical,
                        parameter.default_range,
                    )?;
                }
            }
            HeaderKind::Struct { fields, .. } => {
                for field in fields {
                    append_source_range_paths(
                        &mut consumed_path_syntax_ids,
                        canonical,
                        field.default_range,
                    )?;
                }
            }
            HeaderKind::Choice { variants, .. } => {
                for variant in variants {
                    let crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Record {
                        fields,
                    } = &variant.payload
                    else {
                        continue;
                    };
                    for field in fields {
                        append_source_range_paths(
                            &mut consumed_path_syntax_ids,
                            canonical,
                            field.default_range,
                        )?;
                    }
                }
            }
            HeaderKind::Trait { declaration } => {
                for requirement in &declaration.requirements {
                    for parameter in &requirement.signature.parameters {
                        append_source_range_paths(
                            &mut consumed_path_syntax_ids,
                            canonical,
                            parameter.default_range,
                        )?;
                    }
                }
            }
            HeaderKind::StartFunction
            | HeaderKind::TypeAlias { .. }
            | HeaderKind::ConstTemplate { .. }
            | HeaderKind::TraitConformance { .. }
            | HeaderKind::TraitIncompatibility { .. } => {}
        }
    }

    output.structural_file_references = classify_prepared_file_references(
        output.path_syntax.table(),
        consumed_path_syntax_ids,
        output.file_id,
        path_fork,
        string_table,
    );

    collect_content_source_ordering_hints(
        &mut output.headers,
        canonical,
        &output.structural_file_references,
        output.path_syntax.table(),
        string_table,
        path_fork,
    )
}

/// Scan one retained source-owned range for a `#Config` marker without materializing it.
///
/// The structural fact pass only needs marker presence; placement/adjacency diagnostics remain
/// on the span-aware `find_config_qualifier_marker_in_cursor` path used by start-body validation.
fn source_cursor_contains_config_marker(
    mut cursor: TokenCursor<'_>,
    string_table: &StringTable,
) -> bool {
    let Some(mut previous) = cursor.advance() else {
        return false;
    };
    loop {
        let crosses_segment = cursor.is_at_segment_start();
        let Some(current) = cursor.advance() else {
            break;
        };
        if !crosses_segment
            && previous.tag() == crate::compiler_frontend::tokenizer::tokens::TokenTag::HASH
            && current.tag() == crate::compiler_frontend::tokenizer::tokens::TokenTag::SYMBOL
            && current
                .string_id()
                .is_some_and(|name| string_table.resolve(name) == "Config")
        {
            return true;
        }
        if current.is_eof() {
            break;
        }
        previous = current;
    }
    false
}

/// Validate generic parameter names against every dependency binding retained by this file.
///
/// WHAT: applies the file-wide dependency collision policy after header parsing has retained all
///       clauses and declaration shells, regardless of their source order.
/// WHY: dependency visibility is independent of declaration position, while header dispatch must
///       parse generic syntax before later dependency clauses have been encountered.
fn dependency_generic_parameter_collision(
    headers: &[Header],
    dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Option<CompilerDiagnostic> {
    let forbidden_names = dependency_generic_parameter_forbidden_names(
        dependency_clauses,
        dependency_selections,
        string_table,
        path_fork,
    );

    for header in headers {
        let generic_parameters = match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => generic_parameters,
            _ => continue,
        };

        for parameter in &generic_parameters.parameters {
            if forbidden_names.contains(&parameter.name) {
                return Some(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::GenericParameterNameCollision {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.span,
                ));
            }
        }
    }

    None
}

fn dependency_generic_parameter_forbidden_names(
    dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> FxHashSet<StringId> {
    let mut forbidden_names = FxHashSet::default();

    for clause in dependency_clauses {
        let selections = clause
            .selections(dependency_selections)
            .expect("validated file dependency selection range");
        if selections.is_empty() {
            if let Some(local_name) = clause.effective_namespace_local_name(string_table, path_fork)
            {
                forbidden_names.insert(local_name);
            }
        } else {
            for selection in selections {
                forbidden_names.insert(selection.local_name());
            }
        }
    }

    forbidden_names
}

#[cfg(test)]
#[path = "tests/structural_file_reference_tests.rs"]
mod structural_file_reference_tests;
