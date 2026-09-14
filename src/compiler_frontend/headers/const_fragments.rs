//! Top-level const-template header creation.
//!
//! WHAT: turns entry-file `#[...]` templates into const-template headers plus placement metadata.
//! WHY: const fragments are folded by AST but ordered by header parsing through runtime insertion
//! indices, so this logic must stay in the header stage.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::ordering_hints::dependency_path_for_local_name;
use crate::compiler_frontend::headers::types::{
    Header, HeaderBuildContext, HeaderExportMode, HeaderKind, HeaderParseFailure,
    LocalDeclarationOrderingHint,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanJoinError,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenIndex, TokenKind, TokenRange};
use crate::compiler_frontend::utilities::token_scan::{
    InitializerReference, NestingDepth, collect_symbol_references,
};
use crate::projects::settings::TOP_LEVEL_CONST_TEMPLATE_NAME;
use std::collections::HashSet;

pub(super) fn create_top_level_const_template(
    scope: PathId,
    opening_template_token: Token,
    const_template_number: usize,
    token_stream: &mut FileTokens,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<Header, HeaderParseFailure> {
    let const_template_name = context.string_table.intern(&format!(
        "{TOP_LEVEL_CONST_TEMPLATE_NAME}{const_template_number}"
    ));
    let mut local_ordering_hints: HashSet<LocalDeclarationOrderingHint> = HashSet::new();
    let mut selection_error = None;

    // Keep a bounded temporary scan buffer for dependency facts only; the retained template body
    // is represented by a source-qualified range below.
    let body_start = token_stream.index.checked_sub(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template opening token precedes the source token index",
        ))
    })?;
    let mut body = Vec::with_capacity(10);
    body.push(opening_template_token);
    let start_span = SourceSpan::new(
        token_stream.file_id,
        token_stream.tokens[token_stream.index].span,
    );
    let source_start = SourceSpan::new(token_stream.file_id, LocalSpan::source_start());
    let closing_bracket = context.string_table.intern("]");
    crate::compiler_frontend::utilities::token_scan::consume_balanced_template_region(
        token_stream,
        |token, token_kind| {
            if selection_error.is_none()
                && let TokenKind::Symbol(name_id) = token_kind
            {
                match dependency_path_for_local_name(
                    *name_id,
                    context.file_dependency_clauses,
                    context.dependency_selections,
                    context.string_table,
                    context.path_fork,
                ) {
                    Ok(Some(path)) => {
                        local_ordering_hints
                            .insert(LocalDeclarationOrderingHint::provider_spelling(path));
                    }
                    Ok(None) => {}
                    Err(error) => selection_error = Some(error),
                }
            }
            body.push(token);
        },
        |span| CompilerDiagnostic::unexpected_end_of_file(Some(closing_bracket), Some(span)),
    )?;

    if let Some(error) = selection_error {
        return Err(error.into());
    }

    let end_span = SourceSpan::new(token_stream.file_id, token_stream.current_span().local());
    let condition_references =
        collect_template_if_condition_references(&body, token_stream.file_id);

    let full_name = context
        .path_fork
        .try_intern_child(scope, const_template_name)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "path table exhausted while interning a const-template path",
            )
        })?;
    // Placement metadata retains the same range as the header: first interior token through
    // the post-close token. A long join appends to this source's original extended table.
    let name_span = start_span
        .join(end_span, span_builder)
        .map_err(|error| match error {
            SpanJoinError::Capacity(error) => {
                match CompilerDiagnostic::from_span_capacity_error(error, Some(source_start)) {
                    Ok(diagnostic) => HeaderParseFailure::Diagnostic(diagnostic),
                    Err(error) => HeaderParseFailure::Infrastructure(error),
                }
            }
            SpanJoinError::DifferentSources { .. } => HeaderParseFailure::Infrastructure(
                CompilerError::compiler_error("const-template span join crossed source identities"),
            ),
        })?;

    let start = TokenIndex::try_from_index(body_start).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template body start exceeded the source token index space",
        ))
    })?;
    let mut body_end = token_stream.index;
    // Preserve the source EOF sentinel when this template is the final top-level item. The
    // parser adapter still receives only this contiguous source window; templates followed by
    // another declaration stop at the first post-close token instead of consuming that item.
    if matches!(token_stream.current_token_kind(), TokenKind::Eof) {
        body_end = body_end.checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "const-template body end exceeded the source token index space",
            ))
        })?;
    } else if matches!(token_stream.current_token_kind(), TokenKind::Newline)
        && matches!(token_stream.peek_next_token(), Some(TokenKind::Eof))
    {
        body_end = body_end.checked_add(2).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "const-template body end exceeded the source token index space",
            ))
        })?;
    }
    let end = TokenIndex::try_from_index(body_end).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template body end exceeded the source token index space",
        ))
    })?;
    let template_range = TokenRange::new(token_stream.file_id, start, end).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template body range was reversed",
        ))
    })?;

    Ok(Header {
        kind: HeaderKind::ConstTemplate {
            condition_references,
        },
        file_role: context.file_role,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints,
        name_span: Some(name_span),
        tokens: template_range,
        declaration_path: full_name,
        transitional_tokens: None,
        capacity_references: Vec::new(),
    })
}

fn collect_template_if_condition_references(
    tokens: &[Token],
    source_id: SourceId,
) -> Vec<InitializerReference> {
    let mut references = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        if matches!(tokens[index].kind, TokenKind::If) {
            let condition_start = index + 1;
            let condition_end = find_template_if_condition_end(tokens, condition_start);
            references.extend(collect_symbol_references(
                &tokens[condition_start..condition_end],
                source_id,
            ));
            index = condition_end;
            continue;
        }

        index += 1;
    }

    references
}

fn find_template_if_condition_end(tokens: &[Token], start: usize) -> usize {
    let mut nesting_depth = NestingDepth::default();
    let mut index = start;

    while index < tokens.len() {
        let token = &tokens[index];

        if nesting_depth.is_top_level() {
            match token.kind {
                // Template suffix bodies use StartTemplateBody. Colon is included for
                // defensive parity with other header scanners and tests.
                TokenKind::StartTemplateBody
                | TokenKind::Colon
                | TokenKind::TemplateClose
                | TokenKind::Eof => return index,

                // Option-present template `if` conditions only depend on the scrutinee
                // before `is`; capture names are branch-local, not header deps.
                TokenKind::Is => return index,

                _ => {}
            }
        }

        nesting_depth.step(&token.kind);
        index += 1;
    }

    index
}
