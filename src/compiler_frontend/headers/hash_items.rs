//! Top-level `#` item handling for header parsing.
//!
//! WHAT: handles boundary `#` items, currently the active-root const-template form (`#[]`).
//! WHY: the parser keeps hash-prefixed top-level forms in one place so `file_parser` can remain a
//! high-level loop over classified items.

use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, InvalidConfigReason,
};
use crate::compiler_frontend::declaration_syntax::build_config_contract::find_config_qualifier_marker;
use crate::compiler_frontend::headers::const_fragments::create_top_level_const_template;
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::types::{
    FileRole, HeaderBuildContext, HeaderParseContext, HeaderParseFailure, TopLevelConstFragment,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};

pub(super) fn handle_hash_item(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_token: Token,
    current_span: SourceSpan,
    at_statement_boundary: bool,
) -> Result<(), HeaderParseFailure> {
    let current_index = token_stream.index.saturating_sub(1);

    if !at_statement_boundary {
        state.record_start_body_token(token_stream.file_id, current_index, &current_token)?;
        return Ok(());
    }

    match token_stream.current_token_kind() {
        TokenKind::TemplateHead => {
            if state.export_mode.is_public() {
                return Err(CompilerDiagnostic::invalid_export_target(Some(current_span)).into());
            }

            handle_top_level_const_template(token_stream, state, context, current_span)
        }

        _ => {
            state.record_start_body_token(token_stream.file_id, current_index, &current_token)?;
            Ok(())
        }
    }
}

fn handle_top_level_const_template(
    token_stream: &mut FileTokens,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_span: SourceSpan,
) -> Result<(), HeaderParseFailure> {
    if context.file_role == FileRole::Normal {
        return Err(CompilerDiagnostic::deferred_feature(
            context
                .string_table
                .intern("top-level const templates in ordinary source files"),
            Some(current_span),
        )
        .into());
    }

    if context.file_role == FileRole::ImportedModuleRoot {
        let template_index = token_stream.index;
        token_stream.advance();
        let range = crate::compiler_frontend::headers::start_capture::capture_runtime_template_range(
            template_index,
            token_stream,
            context.string_table,
        )?;
        let body_tokens = token_stream
            .tokens
            .get(range.start().index()..range.end().index())
            .ok_or_else(|| {
                HeaderParseFailure::Infrastructure(crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "discarded template range exceeds its source token owner",
                ))
            })?;
        if let Some((marker_span, adjacent)) = find_config_qualifier_marker(
            body_tokens,
            context.string_table,
            token_stream.file_id,
            context.span_builder,
        ) {
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
            return Err(diagnostic.into());
        }
        return Ok(());
    }

    if context.file_role == FileRole::ActiveApiOnlyModuleRoot {
        return Err(
            CompilerDiagnostic::invalid_top_level_runtime_statement(Some(current_span)).into(),
        );
    }

    let template_token = token_stream.current_token();
    token_stream.advance();

    let source_file = token_stream.src_path;
    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file,
        file_dependency_clauses: &state.file_dependency_clauses,
        dependency_selections: &state.dependency_selections,
        string_table: context.string_table,
        path_fork: context.path_fork,
        file_role: context.file_role,
    };
    let header = create_top_level_const_template(
        source_file,
        template_token,
        context.const_template_offset + state.const_template_count,
        token_stream,
        &mut build_context,
        context.span_builder,
    )?;

    state.const_template_count += 1;

    // Record placement metadata: runtime_insertion_index is the count of runtime fragments
    // seen before this const fragment in source order.
    let fragment_path = header.declaration_path;
    let fragment = TopLevelConstFragment {
        runtime_insertion_index: context.runtime_fragment_offset + state.runtime_fragment_count,
        span: header
            .name_span
            .expect("authored const-template headers carry a source span"),
        header_path: fragment_path,
    };
    state.register_top_level_const_fragment(fragment, header);

    Ok(())
}
