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
    if !at_statement_boundary {
        state.push_start_body_token(current_token);
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
            state.push_start_body_token(current_token);
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
        let template_token = token_stream.current_token();
        token_stream.advance();
        let mut discarded_body = Vec::new();

        crate::compiler_frontend::headers::start_capture::push_runtime_template_tokens_to_start_function(
            template_token,
            token_stream,
            &mut discarded_body,
            context.string_table,
        )?;
        if let Some((marker_span, adjacent)) = find_config_qualifier_marker(
            &discarded_body,
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

    let source_file = token_stream.src_path.to_owned();

    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file: &source_file,
        file_dependency_clauses: &state.file_dependency_clauses,
        dependency_selections: &state.dependency_selections,
        string_table: context.string_table,
        file_role: context.file_role,
    };
    let header = create_top_level_const_template(
        token_stream.src_path.to_owned(),
        template_token,
        context.const_template_offset + state.const_template_count,
        token_stream,
        &mut build_context,
        context.span_builder,
    )?;

    state.const_template_count += 1;

    // Record placement metadata: runtime_insertion_index is the count of runtime fragments
    // seen before this const fragment in source order.
    let fragment = TopLevelConstFragment {
        runtime_insertion_index: context.runtime_fragment_offset + state.runtime_fragment_count,
        span: header
            .name_span
            .expect("authored const-template headers carry a source span"),
        header_path: header.tokens.src_path.clone(),
    };
    state.register_top_level_const_fragment(fragment, header);

    Ok(())
}
