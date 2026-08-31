//! Per-file header splitting.
//!
//! WHAT: orchestrates one tokenized Moth file into top-level declaration headers, dependency
//! records, const-fragment metadata, and the implicit entry `start` body.
//! WHY: file-level control flow is different from declaration parsing, dependency recording, and hash
//! item handling; this module keeps the high-level loop visible while delegated modules own details.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword, reserved_trait_keyword_error,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidDeclarationReason, InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::headers::file_dependency_clauses::{
    parse_and_record_private_dependency, parse_and_record_public_dependency,
};
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::hash_items::handle_hash_item;
use crate::compiler_frontend::headers::header_dispatch::create_header;
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::start_capture::push_runtime_template_tokens_to_start_function;
use crate::compiler_frontend::headers::symbol_collection::is_receiver_method_candidate;
use crate::compiler_frontend::headers::top_level_classifier::{
    HeaderFileItem, classify_current_item, classify_export_block_item,
    starts_duplicate_top_level_header_declaration,
    starts_specialized_generic_conformance_declaration, starts_trait_declaration_after_must,
};
use crate::compiler_frontend::headers::types::{
    DependencySelection, FileFrontendPrepareFailure, FileFrontendPrepareOutput, FileRole, Header,
    HeaderBuildContext, HeaderExportMode, HeaderKind, HeaderParseContext, HeaderParseFailure,
    RetainedDependencyClause,
};
use crate::compiler_frontend::paths::const_paths::can_serialize_path_component_bare;
use crate::compiler_frontend::paths::file_references::classify_prepared_file_references;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
use rustc_hash::FxHashSet;

/// Boxed diagnostic result for file-local header-item orchestration.
///
/// WHAT: gives the connected helper family one small error boundary.
/// WHY: the header loop passes structured diagnostics through several item handlers
///      without carrying the large value inline at every return.
type FileParserResult<T> = Result<T, HeaderParseFailure>;

fn diagnostic_failure(diagnostic: CompilerDiagnostic) -> HeaderParseFailure {
    HeaderParseFailure::Diagnostic(Box::new(diagnostic))
}

// Top-level declarations are same-module-visible by default; cross-module public visibility
// comes only from the root `export:` block. Non-declaration statements are collected into the
// implicit start-function header for that file.
pub(super) fn parse_headers_in_file(
    token_stream: &mut FileTokens,
    context: &mut HeaderParseContext<'_>,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let mut state = HeaderFileParseState::new(token_stream.length);

    let result = parse_headers_in_file_inner(token_stream, context, &mut state);

    match result {
        Ok(()) => finish_file_output(token_stream, context, state),
        Err(HeaderParseFailure::Diagnostic(diagnostic)) => Err(
            FileFrontendPrepareFailure::Diagnosed(state.into_error(*diagnostic)),
        ),
        Err(HeaderParseFailure::Infrastructure(error)) => {
            Err(FileFrontendPrepareFailure::Infrastructure(error))
        }
    }
}

fn parse_headers_in_file_inner(
    token_stream: &mut FileTokens,
    context: &mut HeaderParseContext<'_>,
    state: &mut HeaderFileParseState,
) -> FileParserResult<()> {
    loop {
        let current_token = token_stream.current_token();
        let current_location = token_stream.current_location();
        token_stream.advance();

        match classify_current_item(token_stream, &current_token) {
            HeaderFileItem::Symbol(name_id) => {
                handle_symbol_item(
                    token_stream,
                    state,
                    context,
                    current_token,
                    name_id,
                    current_location,
                )?;
            }

            HeaderFileItem::BuiltinTypeConformanceTarget(type_name) => {
                let name_id = context.string_table.intern(type_name);
                handle_symbol_item(
                    token_stream,
                    state,
                    context,
                    current_token,
                    name_id,
                    current_location,
                )?;
            }

            HeaderFileItem::Dependency => {
                parse_and_record_private_dependency(
                    token_stream,
                    state,
                    context,
                    current_location,
                )?;
            }

            HeaderFileItem::Export => {
                reject_non_block_export(token_stream, context, current_location)?;
            }

            HeaderFileItem::ExportBlock => {
                handle_export_block(token_stream, state, context, current_location)?;
            }

            HeaderFileItem::Hash {
                at_statement_boundary,
            } => {
                handle_hash_item(
                    token_stream,
                    state,
                    context,
                    current_token,
                    current_location,
                    at_statement_boundary,
                )?;
            }

            HeaderFileItem::ReservedTraitSyntax => {
                handle_trait_keyword_header_item(&current_token, current_location)?;
            }

            HeaderFileItem::RuntimeTemplate => {
                handle_runtime_template_item(token_stream, state, context, current_token)?;
            }

            HeaderFileItem::Eof => {
                state.push_start_body_token(current_token);
                break;
            }

            HeaderFileItem::StartBodyToken => {
                state.push_start_body_token(current_token);
            }
        }
    }

    Ok(())
}

fn reject_non_block_export(
    token_stream: &mut FileTokens,
    context: &mut HeaderParseContext<'_>,
    export_location: SourceLocation,
) -> FileParserResult<()> {
    // `export` is valid only as the module-root `export:` block.
    if !context.file_role.is_export_capable() || context.is_config_file {
        return Err(diagnostic_failure(
            CompilerDiagnostic::export_outside_module_root(export_location),
        ));
    }

    // Without the block delimiter, the token is not an export target. Keep this diagnostic in
    // header parsing instead of interpreting the following tokens through another syntax path.
    Err(diagnostic_failure(CompilerDiagnostic::expected_token(
        TokenKind::Colon,
        Some(token_stream.current_token_kind().to_owned()),
        export_location,
    )))
}

fn handle_export_block(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    export_location: SourceLocation,
) -> FileParserResult<()> {
    if !context.file_role.is_export_capable() || context.is_config_file {
        return Err(diagnostic_failure(
            CompilerDiagnostic::export_outside_module_root(export_location),
        ));
    }

    if state.seen_export_block.is_some() {
        return Err(diagnostic_failure(
            CompilerDiagnostic::duplicate_export_block(export_location),
        ));
    }

    // The classifier only produces ExportBlock when the current token is `:`, but consume it
    // here so the item parser starts at the first ordinary top-level item.
    if token_stream.current_token_kind() != &TokenKind::Colon {
        return Err(diagnostic_failure(CompilerDiagnostic::expected_token(
            TokenKind::Colon,
            Some(token_stream.current_token_kind().to_owned()),
            export_location,
        )));
    }
    state.seen_export_block = Some(export_location.clone());
    state.export_mode = HeaderExportMode::Public;
    token_stream.advance();

    while !matches!(
        token_stream.current_token_kind(),
        TokenKind::End | TokenKind::Eof
    ) {
        if token_stream.current_token_kind() == &TokenKind::Newline {
            token_stream.advance();
            continue;
        }

        let current_token = token_stream.current_token();
        let current_location = token_stream.current_location();
        token_stream.advance();

        let item = classify_export_block_item(token_stream, &current_token);
        parse_export_block_item(
            token_stream,
            state,
            context,
            item,
            current_token,
            current_location,
        )?;
        state.export_block_item_count += 1;
    }

    if token_stream.current_token_kind() == &TokenKind::Eof {
        return Err(diagnostic_failure(
            CompilerDiagnostic::unexpected_end_of_file(
                Some(context.string_table.intern(";")),
                token_stream.current_location(),
            ),
        ));
    }

    // The block terminator belongs to this parser mode and must not become an implicit start-body
    // token for the surrounding file.
    token_stream.advance();
    state.export_mode = HeaderExportMode::Private;

    if state.export_block_item_count == 0 {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(export_location),
        ));
    }

    Ok(())
}

fn parse_export_block_item(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    item: HeaderFileItem,
    current_token: Token,
    current_location: SourceLocation,
) -> FileParserResult<()> {
    match item {
        HeaderFileItem::Symbol(name_id) => handle_symbol_item(
            token_stream,
            state,
            context,
            current_token,
            name_id,
            current_location,
        ),

        HeaderFileItem::BuiltinTypeConformanceTarget(type_name) => {
            let name_id = context.string_table.intern(type_name);
            handle_symbol_item(
                token_stream,
                state,
                context,
                current_token,
                name_id,
                current_location,
            )
        }

        HeaderFileItem::Dependency => {
            parse_and_record_public_dependency(token_stream, state, context, current_location)
        }

        HeaderFileItem::Export | HeaderFileItem::ExportBlock => Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(current_location),
        )),

        HeaderFileItem::Hash {
            at_statement_boundary,
        } => Ok(handle_hash_item(
            token_stream,
            state,
            context,
            current_token,
            current_location,
            at_statement_boundary,
        )?),

        HeaderFileItem::RuntimeTemplate | HeaderFileItem::StartBodyToken => Err(
            diagnostic_failure(CompilerDiagnostic::invalid_export_target(current_location)),
        ),

        HeaderFileItem::ReservedTraitSyntax => {
            if let Some(keyword) = reserved_trait_keyword(&current_token.kind) {
                return Err(diagnostic_failure(reserved_trait_keyword_error(
                    keyword,
                    current_location,
                )));
            }

            Err(diagnostic_failure(
                CompilerDiagnostic::invalid_export_target(current_location),
            ))
        }

        HeaderFileItem::Eof => Err(diagnostic_failure(
            CompilerDiagnostic::unexpected_end_of_file(
                Some(context.string_table.intern(";")),
                current_location,
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
    import_index: usize,
    path_index: usize,
}

/// Recognise only `import`, optional newlines, then a path token.
///
/// Comments are not produced as tokens, so comments between the keyword and path are
/// already transparent. `import = 1` and `import` followed by an unrelated statement
/// do not match.
fn recognize_legacy_dependency_start(
    tokens: &[Token],
    import_index: usize,
    string_table: &StringTable,
) -> Option<LegacyDependencyStart> {
    let import = tokens.get(import_index)?;
    let TokenKind::Symbol(name) = import.kind else {
        return None;
    };
    if string_table.resolve(name) != "import" {
        return None;
    }

    let mut index = import_index + 1;
    while tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Newline)
    {
        index += 1;
    }

    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::Path(_)) => Some(LegacyDependencyStart {
            import_index,
            path_index: index,
        }),
        _ => None,
    }
}

fn handle_symbol_item(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_token: Token,
    name_id: StringId,
    current_location: SourceLocation,
) -> FileParserResult<()> {
    let import_index = token_stream.index.saturating_sub(1);
    if let Some(start) =
        recognize_legacy_dependency_start(&token_stream.tokens, import_index, context.string_table)
    {
        return Err(diagnostic_failure(legacy_dependency_clause_diagnostic(
            token_stream,
            context.string_table,
            current_location,
            context.is_config_file,
            start,
        )?));
    }

    let export_mode = state.export_mode;
    handle_symbol_item_with_export_mode(
        token_stream,
        state,
        context,
        current_token,
        name_id,
        current_location,
        export_mode,
    )
}

/// Build the one-way migration diagnostic for the removed `import @...` grammar.
///
/// A replacement is offered only when the old clause maps mechanically to one current clause.
/// Filtered namespaces and nested groups deliberately require an author choice.
fn legacy_dependency_clause_diagnostic(
    token_stream: &FileTokens,
    string_table: &mut StringTable,
    current_location: SourceLocation,
    is_config_file: bool,
    start: LegacyDependencyStart,
) -> Result<CompilerDiagnostic, HeaderParseFailure> {
    let mut clause_location = token_stream
        .tokens
        .get(start.import_index)
        .map(|token| token.location.clone())
        .unwrap_or(current_location);
    let clause_end = legacy_dependency_clause_end(&token_stream.tokens, start.path_index)
        .and_then(|index| token_stream.tokens.get(index))
        .map_or(clause_location.end_pos, |token| token.location.end_pos);
    clause_location.end_pos = clause_end;

    let replacement = if is_config_file {
        None
    } else {
        legacy_dependency_replacement(token_stream, string_table, start.path_index)?
            .map(|replacement| string_table.intern(&replacement))
    };
    Ok(CompilerDiagnostic::legacy_dependency_clause(
        replacement,
        clause_location,
    ))
}

fn legacy_dependency_clause_end(tokens: &[Token], path_index: usize) -> Option<usize> {
    let mut index = path_index;
    let mut brace_depth = 0usize;
    let mut last = None;
    while let Some(token) = tokens.get(index) {
        match token.kind {
            TokenKind::OpenCurly => brace_depth += 1,
            TokenKind::CloseCurly => brace_depth = brace_depth.saturating_sub(1),
            TokenKind::Newline if brace_depth == 0 => break,
            TokenKind::End | TokenKind::Eof => break,
            _ => {}
        }
        last = Some(index);
        index += 1;
    }
    last
}

fn legacy_dependency_replacement(
    token_stream: &FileTokens,
    string_table: &StringTable,
    path_index: usize,
) -> Result<Option<String>, HeaderParseFailure> {
    let tokens = &token_stream.tokens;
    let Some(path_token) = tokens.get(path_index) else {
        return Ok(None);
    };
    let TokenKind::Path(path_id) = path_token.kind else {
        return Ok(None);
    };
    let path = token_stream
        .path_syntax_table()?
        .try_path_for_token(path_id, &path_token.location)?
        .root
        .to_owned();
    if path.is_empty() {
        // Exact `@/` is represented by the empty canonical path, and so is the bare introducer
        // `@`. The retained row cannot tell the two spellings apart, so it suggests neither.
        return Ok(None);
    }
    if path
        .as_components()
        .iter()
        .any(|component| !can_serialize_path_component_bare(string_table.resolve(*component)))
    {
        // The canonical path row intentionally does not retain component quoting. Do not emit a
        // normalized spelling that would turn a valid quoted path into invalid unquoted syntax.
        return Ok(None);
    }
    let path = path.to_portable_string(string_table);
    let mut replacement = format!("@{path}");
    let mut index = path_index + 1;

    if tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::As)
    {
        let Some(Token {
            kind: TokenKind::Symbol(alias),
            ..
        }) = tokens.get(index + 1)
        else {
            return Ok(None);
        };
        replacement.push_str(" as ");
        replacement.push_str(string_table.resolve(*alias));
        index += 2;
        return Ok(clause_terminator(tokens.get(index)).then_some(replacement));
    }

    if !tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::OpenCurly)
    {
        return Ok(clause_terminator(tokens.get(index)).then_some(replacement));
    }
    index += 1;
    replacement.push(' ');

    let mut first_selection = true;
    loop {
        while tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Newline)
        {
            index += 1;
        }
        let Some(token) = tokens.get(index) else {
            return Ok(None);
        };
        if token.kind == TokenKind::CloseCurly {
            return Ok(
                (!first_selection && clause_terminator(tokens.get(index + 1)))
                    .then_some(replacement),
            );
        }
        let TokenKind::Symbol(name) = token.kind else {
            return Ok(None);
        };
        if !first_selection {
            replacement.push_str(", ");
        }
        replacement.push_str(string_table.resolve(name));
        first_selection = false;
        index += 1;

        if tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::As)
        {
            let Some(Token {
                kind: TokenKind::Symbol(alias),
                ..
            }) = tokens.get(index + 1)
            else {
                return Ok(None);
            };
            replacement.push_str(" as ");
            replacement.push_str(string_table.resolve(*alias));
            index += 2;
        }

        match tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Comma) => index += 1,
            Some(TokenKind::CloseCurly | TokenKind::Newline) => {}
            _ => return Ok(None),
        }
    }
}

fn clause_terminator(token: Option<&Token>) -> bool {
    token.is_none_or(|token| {
        matches!(
            token.kind,
            TokenKind::Newline | TokenKind::End | TokenKind::Eof
        )
    })
}

fn handle_symbol_item_with_export_mode(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_token: Token,
    name_id: StringId,
    current_location: SourceLocation,
    export_mode: HeaderExportMode,
) -> FileParserResult<()> {
    if export_mode.is_public() && !starts_duplicate_top_level_header_declaration(token_stream) {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(current_location),
        ));
    }

    if let Some(first_location) = state.encountered_symbols.get(&name_id) {
        let is_conformance_declaration = (token_stream.current_token_kind() == &TokenKind::Must
            && !starts_trait_declaration_after_must(token_stream))
            || starts_specialized_generic_conformance_declaration(token_stream);

        // Conformance declarations reuse the target type name (`Type must TRAIT`).
        // They do not conflict with the type declaration itself.
        // AST evidence validation catches duplicate semantic conformance facts later.
        if !is_conformance_declaration
            && starts_duplicate_top_level_header_declaration(token_stream)
        {
            return Err(diagnostic_failure(
                CompilerDiagnostic::duplicate_declaration(
                    name_id,
                    Some(first_location.clone()),
                    current_location,
                ),
            ));
        }

        if !is_conformance_declaration {
            state.push_start_body_token(current_token);
            // Body-level symbol/dependency resolution belongs to AST passes. Header parsing only validates
            // duplicate top-level declaration starts at this stage.
            return Ok(());
        }

        // Fall through for conformance declarations so they are parsed as real headers.
    }

    if state.start_body_symbols.contains(&name_id)
        && !starts_duplicate_top_level_header_declaration(token_stream)
    {
        state.push_start_body_token(current_token);
        return Ok(());
    }

    let source_file = token_stream.src_path.to_owned();
    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file: &source_file,
        file_dependency_clauses: &state.file_dependency_clauses,
        dependency_selections: &state.dependency_selections,
        string_table: context.string_table,
        file_role: context.file_role,
    };
    let header = create_header(
        token_stream.src_path.append(name_id),
        token_stream,
        current_location.clone(),
        export_mode,
        &mut build_context,
    )?;

    if export_mode.is_public()
        && matches!(
            &header.kind,
            HeaderKind::StartFunction
                | HeaderKind::TraitConformance { .. }
                | HeaderKind::TraitIncompatibility { .. }
        )
    {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_export_target(current_location),
        ));
    }

    if export_mode.is_public()
        && let HeaderKind::Function { signature, .. } = &header.kind
        && is_receiver_method_candidate(signature, context.string_table)
    {
        return Err(diagnostic_failure(
            CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
                current_location,
            ),
        ));
    }

    match header.kind {
        HeaderKind::StartFunction => {
            state.push_start_body_token(current_token);
            state.register_start_body_symbol(name_id);
        }
        HeaderKind::TraitConformance { .. } | HeaderKind::TraitIncompatibility { .. } => {
            // Conformance and incompatibility declarations reuse an existing target/subject
            // name and must not shadow that name's entry in encountered_symbols for duplicate
            // detection.
            state.register_header(header);
        }

        _ => {
            let name_location = header.name_location.clone();
            state.register_header(header);
            state.encountered_symbols.insert(name_id, name_location);
        }
    }

    Ok(())
}

fn handle_trait_keyword_header_item(
    current_token: &Token,
    current_location: SourceLocation,
) -> FileParserResult<()> {
    if let Some(keyword) = reserved_trait_keyword(&current_token.kind) {
        return Err(diagnostic_failure(reserved_trait_keyword_error(
            keyword,
            current_location,
        )));
    }

    Ok(())
}

fn handle_runtime_template_item(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_token: Token,
) -> FileParserResult<()> {
    // Runtime top-level templates stay in the start-function body and are evaluated in source
    // order by entry start(). The runtime fragment count lets later const fragments record their
    // insertion point relative to already-seen runtime fragments.
    push_runtime_template_tokens_to_start_function(
        current_token,
        token_stream,
        &mut state.start_function_body,
        context.string_table,
    )?;

    if context.file_role == FileRole::ActiveModuleRoot {
        state.runtime_fragment_count += 1;
    }

    Ok(())
}

fn finish_file_output(
    token_stream: &mut FileTokens,
    context: &mut HeaderParseContext<'_>,
    state: HeaderFileParseState,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    if let Some(diagnostic) = dependency_generic_parameter_collision(
        &state.headers,
        &state.file_dependency_clauses,
        &state.dependency_selections,
        context.string_table,
    ) {
        return Err(FileFrontendPrepareFailure::Diagnosed(
            state.into_error(diagnostic),
        ));
    }

    // Ordinary source files have no semantic consumer for an implicit start. Dependency-reached roots are
    // intentionally parsed for declarations and exports only; their root body is discarded.
    if matches!(
        context.file_role,
        FileRole::Normal | FileRole::ActiveApiOnlyModuleRoot
    ) && state.has_non_trivial_start_body()
    {
        let location = state
            .first_executable_start_body_location()
            .unwrap_or_default();
        return Err(FileFrontendPrepareFailure::Diagnosed(state.into_error(
            CompilerDiagnostic::invalid_top_level_runtime_statement(location),
        )));
    }

    let mut output = if context.file_role == FileRole::ActiveModuleRoot {
        state
            .into_entry_output(token_stream, context.file_role)
            .map_err(FileFrontendPrepareFailure::Infrastructure)?
    } else {
        state
            .into_non_entry_output(token_stream, context.file_role)
            .map_err(FileFrontendPrepareFailure::Infrastructure)?
    };
    attach_structural_file_facts(&mut output, context.string_table)
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
fn attach_structural_file_facts(
    output: &mut FileFrontendPrepareOutput,
    string_table: &mut StringTable,
) -> Result<(), CompilerError> {
    output.structural_file_references = classify_prepared_file_references(
        output.path_syntax.table(),
        output
            .file_dependency_clauses
            .iter()
            .map(|clause| clause.dependency.path_syntax),
        output.file_id,
        string_table,
    );

    collect_content_source_ordering_hints(
        &mut output.headers,
        &output.structural_file_references,
        output.path_syntax.table(),
        string_table,
    )
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
) -> Option<CompilerDiagnostic> {
    let forbidden_names = dependency_generic_parameter_forbidden_names(
        dependency_clauses,
        dependency_selections,
        string_table,
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
                    parameter.location.clone(),
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
) -> FxHashSet<StringId> {
    let mut forbidden_names = FxHashSet::default();

    for clause in dependency_clauses {
        let selections = clause
            .selections(dependency_selections)
            .expect("validated file dependency selection range");
        if selections.is_empty() {
            if let Some(local_name) = clause.effective_namespace_local_name(string_table) {
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
