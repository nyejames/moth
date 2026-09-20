//! Top-level `#` item handling for header parsing.
//!
//! WHAT: handles boundary `#` items, currently the active-root const-template form (`#[]`).
//! WHY: the parser keeps hash-prefixed top-level forms in one place so `file_parser` can remain a
//! high-level loop over classified items.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, InvalidConfigReason,
};
use crate::compiler_frontend::declaration_syntax::build_config_contract::find_config_qualifier_marker_in_cursor;
use crate::compiler_frontend::headers::const_fragments::create_top_level_const_template;
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::start_capture::capture_runtime_template_range_from_cursor;
use crate::compiler_frontend::headers::types::{
    FileRole, HeaderBuildContext, HeaderParseContext, HeaderParseFailure, TopLevelConstFragment,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tokenizer::tokens::{TokenCursor, TokenIndex, TokenTag};

fn source_tag_at_cursor(
    cursor: &TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
) -> Result<TokenTag, HeaderParseFailure> {
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(
                "hash item source token owner does not match its file identity",
            ),
        ));
    }
    cursor.current().map(|token| token.tag()).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "hash item cursor exceeded its source token owner",
        ))
    })
}

fn record_start_body_token_from_source(
    state: &mut HeaderFileParseState,
    cursor: &TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
    index: TokenIndex,
) -> Result<(), HeaderParseFailure> {
    let source_tokens = cursor.source_tokens();
    if source_tokens.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(
                "hash item source token owner does not match its file identity",
            ),
        ));
    }
    let token = source_tokens.token(index).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "hash item token index exceeded its source token owner: {error:?}",
        )))
    })?;
    state
        .record_start_body_token_ref(token)
        .map_err(HeaderParseFailure::Infrastructure)
}

pub(super) struct HashItemRequest {
    pub(super) current_index: TokenIndex,
    pub(super) current_span: SourceSpan,
    pub(super) at_statement_boundary: bool,
}

pub(super) fn handle_hash_item(
    cursor: &mut TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    request: HashItemRequest,
) -> Result<(), HeaderParseFailure> {
    let HashItemRequest {
        current_index,
        current_span,
        at_statement_boundary,
    } = request;
    if !at_statement_boundary {
        record_start_body_token_from_source(state, cursor, file_id, current_index)?;
        return Ok(());
    }

    match source_tag_at_cursor(cursor, file_id)? {
        TokenTag::TEMPLATE_HEAD => {
            if state.export_mode.is_public() {
                return Err(CompilerDiagnostic::invalid_export_target(Some(current_span)).into());
            }

            handle_top_level_const_template(
                cursor,
                file_id,
                source_file,
                state,
                context,
                current_span,
            )
        }

        _ => record_start_body_token_from_source(state, cursor, file_id, current_index),
    }
}

fn handle_top_level_const_template(
    cursor: &mut TokenCursor<'_>,
    file_id: crate::compiler_frontend::source::SourceId,
    source_file: PathId,
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
        let opening = cursor.current().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "runtime template opening exceeded its source token owner",
            ))
        })?;
        cursor.advance();
        let range = capture_runtime_template_range_from_cursor(
            opening,
            cursor,
            file_id,
            context.string_table,
        )?;
        let canonical = cursor.source_tokens();
        let cursor = canonical.cursor(range).map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "discarded template range exceeds its source token owner: {error:?}",
            )))
        })?;
        if let Some((marker_span, adjacent)) = find_config_qualifier_marker_in_cursor(
            cursor,
            context.string_table,
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

    let opening = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template opening exceeded its source token owner",
        ))
    })?;
    cursor.advance();

    let const_template_number = context
        .const_template_offset
        .checked_add(state.const_template_count)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "const-template number overflowed its module source range",
            ))
        })?;

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
        opening,
        const_template_number,
        cursor,
        &mut build_context,
        context.span_builder,
    )?;

    state.const_template_count = state.const_template_count.checked_add(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template count overflowed its source state",
        ))
    })?;

    // Record placement metadata: runtime_insertion_index is the count of runtime fragments
    // seen before this const fragment in source order.
    let runtime_insertion_index = context
        .runtime_fragment_offset
        .checked_add(state.runtime_fragment_count)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "runtime insertion index overflowed its module source range",
            ))
        })?;
    let fragment_path = header.declaration_path;
    let fragment = TopLevelConstFragment {
        runtime_insertion_index,
        span: header
            .name_span
            .expect("authored const-template headers carry a source span"),
        header_path: fragment_path,
    };
    state.register_top_level_const_fragment(fragment, header);

    Ok(())
}
