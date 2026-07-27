//! Top-level `#` item handling for header parsing.
//!
//! WHAT: handles boundary `#` items, currently the active-root const-template form (`#[]`).
//! WHY: the parser keeps hash-prefixed top-level forms in one place so `file_parser` can remain a
//! high-level loop over classified items.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::const_fragments::create_top_level_const_template;
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::types::{
    FileRole, HeaderBuildContext, HeaderParseContext, TopLevelConstFragment,
};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};

/// Boxed diagnostic result for hash-item handling.
///
/// WHAT: gives the hash-item family one small error boundary.
/// WHY: hash-item parsing passes structured diagnostics through to the file-parser loop
///      without carrying the large value inline at every return.
type HashItemsResult<T> = Result<T, Box<CompilerDiagnostic>>;

pub(super) fn handle_hash_item(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_token: Token,
    current_location: SourceLocation,
    at_statement_boundary: bool,
) -> HashItemsResult<()> {
    if !at_statement_boundary {
        state.push_start_body_token(current_token);
        return Ok(());
    }

    match token_stream.current_token_kind() {
        TokenKind::TemplateHead => {
            if state.export_mode.is_public() {
                return Err(Box::new(CompilerDiagnostic::invalid_export_target(
                    current_location,
                )));
            }

            handle_top_level_const_template(token_stream, state, context, current_location)
        }

        _ => {
            state.push_start_body_token(current_token);
            Ok(())
        }
    }
}

fn handle_top_level_const_template(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_location: SourceLocation,
) -> HashItemsResult<()> {
    if context.file_role == FileRole::Normal {
        return Err(Box::new(CompilerDiagnostic::deferred_feature(
            context
                .string_table
                .intern("top-level const templates in ordinary source files"),
            current_location,
        )));
    }

    if context.file_role == FileRole::ImportedModuleRoot {
        let template_token = token_stream.current_token();
        token_stream.advance();

        let mut discarded_body = Vec::new();
        crate::compiler_frontend::headers::start_capture::push_runtime_template_tokens_to_start_function(
            template_token,
            token_stream,
            &mut discarded_body,
            context.string_table,
        )?;
        return Ok(());
    }

    if context.file_role == FileRole::ActiveApiOnlyModuleRoot {
        return Err(Box::new(
            CompilerDiagnostic::invalid_top_level_runtime_statement(current_location),
        ));
    }

    let template_token = token_stream.current_token();
    token_stream.advance();

    let source_file = token_stream.src_path.to_owned();
    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file: &source_file,
        file_imports: &state.file_import_paths,
        file_import_entries: &state.file_imports,
        string_table: context.string_table,
        file_role: context.file_role,
    };
    let header = create_top_level_const_template(
        token_stream.src_path.to_owned(),
        template_token,
        context.const_template_offset + state.const_template_count,
        token_stream,
        &mut build_context,
    )?;

    state.const_template_count += 1;

    // Record placement metadata: runtime_insertion_index is the count of runtime fragments
    // seen before this const fragment in source order.
    let fragment = TopLevelConstFragment {
        runtime_insertion_index: context.runtime_fragment_offset + state.runtime_fragment_count,
        location: header.name_location.clone(),
        header_path: header.tokens.src_path.clone(),
    };
    state.register_top_level_const_fragment(fragment, header);

    Ok(())
}
