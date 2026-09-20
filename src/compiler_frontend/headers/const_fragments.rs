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
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
};
use crate::compiler_frontend::utilities::token_scan::{
    InitializerReference, NestingDepth, TokenFactView, collect_scanned_symbol_references,
};
use crate::projects::settings::TOP_LEVEL_CONST_TEMPLATE_NAME;
use std::collections::HashSet;

fn canonical_token_at<'a>(
    canonical: &'a SourceTokens,
    file_id: SourceId,
    index: usize,
    owner: &'static str,
) -> Result<TokenRef<'a>, HeaderParseFailure> {
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(format!(
                "{owner} source token owner does not match its file identity"
            )),
        ));
    }
    let position = TokenIndex::try_from_index(index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "{owner} token index exceeded its checked domain",
        )))
    })?;
    canonical.token(position).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "{owner} token index exceeded its source token owner: {error:?}",
        )))
    })
}

pub(super) fn create_top_level_const_template(
    scope: PathId,
    opening: TokenRef<'_>,
    const_template_number: usize,
    cursor: &mut TokenCursor<'_>,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<Header, HeaderParseFailure> {
    let const_template_name = context.string_table.intern(&format!(
        "{TOP_LEVEL_CONST_TEMPLATE_NAME}{const_template_number}"
    ));
    let mut local_ordering_hints: HashSet<LocalDeclarationOrderingHint> = HashSet::new();
    let file_id = opening.source();
    let source_start = SourceSpan::new(file_id, LocalSpan::source_start());
    let first_body_index = opening.index().index().checked_add(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template first body index exceeded the source token index space",
        ))
    })?;
    let canonical = cursor.source_tokens();
    let start_span = canonical_token_at(
        canonical,
        file_id,
        first_body_index,
        "const-template first body token",
    )?
    .source_span();
    let closing_bracket = context.string_table.intern("]");
    let post_close = crate::compiler_frontend::utilities::token_scan::consume_balanced_template_region_from_source(
        cursor,
        opening,
        |_token| {},
        |span| {
            HeaderParseFailure::Diagnostic(CompilerDiagnostic::unexpected_end_of_file(
                Some(closing_bracket),
                Some(span),
            ))
        },
        HeaderParseFailure::Infrastructure,
    )?;
    let opening_index = opening.index();
    let template_range = canonical
        .range(opening_index, post_close)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "const-template retained range exceeded its source owner: {error:?}",
            )))
        })?;
    let template_facts =
        TokenFactView::from_source_range(canonical, template_range).map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "const-template fact range exceeded its source owner: {error:?}",
            )))
        })?;

    for index in 0..template_facts.len() {
        let Some(token) = template_facts.get(index) else {
            continue;
        };
        if let Some(name_id) = token.symbol.filter(|_| token.tag == TokenTag::SYMBOL) {
            match dependency_path_for_local_name(
                name_id,
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
                Err(error) => return Err(error.into()),
            }
        }
    }

    let end_span = canonical
        .token(post_close)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "const-template post-close index exceeded its source owner: {error:?}",
            )))
        })?
        .source_span();
    let condition_references = collect_template_if_condition_references(template_facts, file_id);

    let full_name = context
        .path_fork
        .try_intern_child(scope, const_template_name)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "path table exhausted while interning a const-template path",
            )
        })?;
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

    let mut body_end = post_close.index();
    let post_close_tag = canonical
        .token(post_close)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "const-template post-close index exceeded its source owner: {error:?}",
            )))
        })?
        .tag();
    if post_close_tag == TokenTag::EOF {
        body_end = body_end.checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "const-template body end exceeded the source token index space",
            ))
        })?;
    } else if post_close_tag == TokenTag::NEWLINE {
        let next_index = post_close.index().checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "const-template newline follower exceeded the source token index space",
            ))
        })?;
        if canonical_token_at(
            canonical,
            file_id,
            next_index,
            "const-template EOF follower",
        )?
        .tag()
            == TokenTag::EOF
        {
            body_end = body_end.checked_add(2).ok_or_else(|| {
                HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                    "const-template body end exceeded the source token index space",
                ))
            })?;
        }
    }
    let end = TokenIndex::try_from_index(body_end).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template body end exceeded the source token index space",
        ))
    })?;
    let retained_range = TokenRange::new(file_id, opening_index, end).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "const-template body range was reversed",
        ))
    })?;
    canonical
        .range(retained_range.start(), retained_range.end())
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "const-template body range exceeded its source owner: {error:?}",
            )))
        })?;

    Ok(Header {
        kind: HeaderKind::ConstTemplate {
            condition_references,
        },
        file_role: context.file_role,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints,
        name_span: Some(name_span),
        synthetic_content_payload: None,
        tokens: retained_range,
        declaration_path: full_name,
        token_sequence: None,
        capacity_references: Vec::new(),
    })
}

fn collect_template_if_condition_references(
    tokens: TokenFactView<'_>,
    source_id: SourceId,
) -> Vec<InitializerReference> {
    let mut references = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let Some(token) = tokens.get(index) else {
            break;
        };
        if token.tag == TokenTag::IF {
            let Some(condition_start) = index.checked_add(1) else {
                break;
            };
            let condition_end = find_template_if_condition_end(tokens, condition_start);
            if let Some(condition) = tokens.subrange(condition_start, condition_end) {
                references.extend(collect_scanned_symbol_references(condition, source_id));
            }
            index = condition_end;
            continue;
        }

        index = index.saturating_add(1);
    }

    references
}

fn find_template_if_condition_end(tokens: TokenFactView<'_>, start: usize) -> usize {
    let mut nesting_depth = NestingDepth::default();
    let mut index = start;

    while index < tokens.len() {
        let Some(token) = tokens.get(index) else {
            return index;
        };

        if nesting_depth.is_top_level() {
            match token.tag {
                // Template suffix bodies use StartTemplateBody. Colon is included for
                // defensive parity with other header scanners and tests.
                TokenTag::START_TEMPLATE_BODY
                | TokenTag::COLON
                | TokenTag::TEMPLATE_CLOSE
                | TokenTag::EOF => return index,

                // Option-present template `if` conditions only depend on the scrutinee
                // before `is`; capture names are branch-local, not header deps.
                TokenTag::IS => return index,

                _ => {}
            }
        }

        nesting_depth.step_tag(token.tag);
        index = index.saturating_add(1);
    }

    index
}
