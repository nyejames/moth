//! Per-file header splitting.
//!
//! WHAT: orchestrates one tokenized Moth file into top-level declaration headers, import
//! records, const-fragment metadata, and the implicit entry `start` body.
//! WHY: file-level control flow is different from declaration parsing, import recording, and hash
//! item handling; this module keeps the high-level loop visible while delegated modules own details.

use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword, reserved_trait_keyword_error,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::headers::file_imports::{
    parse_and_record_imports, parse_and_record_public_block_imports,
};
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::hash_items::handle_hash_item;
use crate::compiler_frontend::headers::header_dispatch::create_header;
use crate::compiler_frontend::headers::start_capture::push_runtime_template_tokens_to_start_function;
use crate::compiler_frontend::headers::symbol_collection::is_receiver_method_candidate;
use crate::compiler_frontend::headers::top_level_classifier::{
    HeaderFileItem, classify_current_item, classify_export_block_item,
    starts_duplicate_top_level_header_declaration,
    starts_specialized_generic_conformance_declaration, starts_trait_declaration_after_must,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareError, FileFrontendPrepareOutput, FileRole, HeaderBuildContext,
    HeaderExportMode, HeaderKind, HeaderParseContext,
};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};

/// Boxed diagnostic result for file-local header-item orchestration.
///
/// WHAT: gives the connected helper family one small error boundary.
/// WHY: the header loop passes structured diagnostics through several item handlers
///      without carrying the large value inline at every return.
type FileParserResult<T> = Result<T, Box<CompilerDiagnostic>>;

// Top-level declarations are same-module-visible by default; cross-module public visibility
// comes only from the root `export:` block. Non-declaration statements are collected into the
// implicit start-function header for that file.
pub(super) fn parse_headers_in_file(
    token_stream: &mut FileTokens,
    context: &mut HeaderParseContext<'_>,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareError> {
    let mut state = HeaderFileParseState::new(token_stream.length);

    let result = parse_headers_in_file_inner(token_stream, context, &mut state);

    match result {
        Ok(()) => finish_file_output(token_stream, context, state),
        Err(boxed_diagnostic) => Err(state.into_error(*boxed_diagnostic)),
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

            HeaderFileItem::Import => {
                parse_and_record_imports(token_stream, state, context, current_location)?;
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
        return Err(Box::new(CompilerDiagnostic::export_outside_module_root(
            export_location,
        )));
    }

    // Without the block delimiter, the token is not an export target. Keep this diagnostic in
    // header parsing instead of interpreting the following tokens through another syntax path.
    Err(Box::new(CompilerDiagnostic::expected_token(
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
        return Err(Box::new(CompilerDiagnostic::export_outside_module_root(
            export_location,
        )));
    }

    if state.seen_export_block.is_some() {
        return Err(Box::new(CompilerDiagnostic::duplicate_export_block(
            export_location,
        )));
    }

    // The classifier only produces ExportBlock when the current token is `:`, but consume it
    // here so the item parser starts at the first ordinary top-level item.
    if token_stream.current_token_kind() != &TokenKind::Colon {
        return Err(Box::new(CompilerDiagnostic::expected_token(
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
        return Err(Box::new(CompilerDiagnostic::unexpected_end_of_file(
            Some(context.string_table.intern(";")),
            token_stream.current_location(),
        )));
    }

    // The block terminator belongs to this parser mode and must not become an implicit start-body
    // token for the surrounding file.
    token_stream.advance();
    state.export_mode = HeaderExportMode::Private;

    if state.export_block_item_count == 0 {
        return Err(Box::new(CompilerDiagnostic::invalid_export_target(
            export_location,
        )));
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

        HeaderFileItem::Import => {
            parse_and_record_public_block_imports(token_stream, state, context, current_location)
        }

        HeaderFileItem::Export | HeaderFileItem::ExportBlock => Err(Box::new(
            CompilerDiagnostic::invalid_export_target(current_location),
        )),

        HeaderFileItem::Hash {
            at_statement_boundary,
        } => handle_hash_item(
            token_stream,
            state,
            context,
            current_token,
            current_location,
            at_statement_boundary,
        ),

        HeaderFileItem::RuntimeTemplate | HeaderFileItem::StartBodyToken => Err(Box::new(
            CompilerDiagnostic::invalid_export_target(current_location),
        )),

        HeaderFileItem::ReservedTraitSyntax => {
            if let Some(keyword) = reserved_trait_keyword(&current_token.kind) {
                return Err(Box::new(reserved_trait_keyword_error(
                    keyword,
                    current_location,
                )));
            }

            Err(Box::new(CompilerDiagnostic::invalid_export_target(
                current_location,
            )))
        }

        HeaderFileItem::Eof => Err(Box::new(CompilerDiagnostic::unexpected_end_of_file(
            Some(context.string_table.intern(";")),
            current_location,
        ))),
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
        return Err(Box::new(CompilerDiagnostic::invalid_export_target(
            current_location,
        )));
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
            return Err(Box::new(CompilerDiagnostic::duplicate_declaration(
                name_id,
                Some(first_location.clone()),
                token_stream.current_location(),
            )));
        }

        if !is_conformance_declaration {
            state.push_start_body_token(current_token);
            // Body-level symbol/import resolution belongs to AST passes. Header parsing only validates
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
        file_imports: &state.file_import_paths,
        file_import_entries: &state.file_imports,
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
        return Err(Box::new(CompilerDiagnostic::invalid_export_target(
            current_location,
        )));
    }

    if export_mode.is_public()
        && let HeaderKind::Function { signature, .. } = &header.kind
        && is_receiver_method_candidate(signature, context.string_table)
    {
        return Err(Box::new(CompilerDiagnostic::invalid_receiver_declaration(
            InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
            current_location,
        )));
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
        return Err(Box::new(reserved_trait_keyword_error(
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
    token_stream: &FileTokens,
    context: &HeaderParseContext<'_>,
    state: HeaderFileParseState,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareError> {
    // Ordinary source files have no semantic consumer for an implicit start. Imported roots are
    // intentionally parsed for declarations and exports only; their root body is discarded.
    if matches!(
        context.file_role,
        FileRole::Normal | FileRole::ActiveApiOnlyModuleRoot
    ) && state.has_non_trivial_start_body()
    {
        let location = state
            .first_executable_start_body_location()
            .unwrap_or_default();
        return Err(
            state.into_error(CompilerDiagnostic::invalid_top_level_runtime_statement(
                location,
            )),
        );
    }

    if context.file_role == FileRole::ActiveModuleRoot {
        Ok(state.into_entry_output(token_stream, context.file_role))
    } else {
        Ok(state.into_non_entry_output(token_stream, context.file_role))
    }
}
