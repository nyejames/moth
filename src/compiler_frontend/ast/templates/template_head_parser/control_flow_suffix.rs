//! Template head suffix parsing for `if` and `loop`.
//!
//! WHAT: turns final template-head control-flow suffixes into a structured body
//! parser mode.
//! WHY: the head parser must recognize control flow before body parsing, but
//! branch/body splitting belongs to the body parser in the next phase.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::statements::if_headers::{ParsedIfHeader, parse_if_header};
use crate::compiler_frontend::ast::statements::loop_headers::{
    ParsedLoopHeader, parse_loop_header_cursor, parse_loop_header_tokens,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBodyParseMode, TemplateBranchSelector, TemplateControlFlowValidationMode,
    TemplateIfBodyParseInput, TemplateLoopBodyParseInput, TemplateLoopHeader,
    inline_source_consts_for_const_required_expression,
    inline_source_consts_for_const_required_if_condition,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

/// Template head control flow joins ordinary expression parsing with template construction. It
/// preserves both authored diagnostics and retained-data infrastructure failures until the
/// template's owning construction boundary.
type ControlFlowSuffixResult<T> = Result<T, TemplateError>;

/// Parse a template `if` suffix after the `if` token has been seen.
pub(crate) fn parse_if_suffix(
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    validation_mode: TemplateControlFlowValidationMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ControlFlowSuffixResult<TemplateBodyParseMode> {
    let marker_span = current_token_local_span(token_stream);
    token_stream.advance(); // consume `if`

    if next_meaningful_token_is_body_boundary(token_stream) {
        return Err(with_token_span(
            token_stream,
            marker_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MissingTemplateIfCondition,
                Some(SourceSpan::new(token_stream.source_id(), marker_span)),
            ),
        )
        .into());
    }

    let parsed_header = parse_if_header(
        token_stream,
        context,
        type_interner,
        string_table,
        path_fork,
    )?;

    ensure_suffix_ends_at_body_start(token_stream)?;
    token_stream.advance(); // consume `:`
    let (mut condition, then_context) = match parsed_header {
        ParsedIfHeader::BoolCondition { condition } => {
            let then_context =
                context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
            (TemplateBranchSelector::Bool(condition), then_context)
        }

        ParsedIfHeader::OptionPresentCapture {
            scrutinee,
            pattern,
            then_context,
        } => {
            let then_context =
                then_context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
            (
                TemplateBranchSelector::OptionPresentCapture {
                    scrutinee,
                    pattern: Box::new(pattern),
                },
                then_context,
            )
        }

        ParsedIfHeader::MatchStyle { scrutinee } => {
            return Err(with_token_span(
                token_stream,
                marker_span,
                CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported,
                    scrutinee.span,
                ),
            )
            .into());
        }
    };

    if validation_mode == TemplateControlFlowValidationMode::ConstRequired {
        condition =
            inline_source_consts_for_const_required_if_condition(condition, context, string_table);
    }

    let else_context = context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);

    Ok(TemplateBodyParseMode::If(Box::new(
        TemplateIfBodyParseInput {
            selector: condition,
            then_context,
            else_context,
            span: Some(SourceSpan::new(token_stream.source_id(), marker_span)),
        },
    )))
}

/// Parse a template `loop` suffix after the `loop` token has been seen.
pub(crate) fn parse_loop_suffix(
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    validation_mode: TemplateControlFlowValidationMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ControlFlowSuffixResult<TemplateBodyParseMode> {
    let marker_span = current_token_local_span(token_stream);
    token_stream.advance(); // consume `loop`

    if next_meaningful_token_is_body_boundary(token_stream) {
        return Err(with_token_span(
            token_stream,
            marker_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MissingTemplateLoopHeader,
                Some(SourceSpan::new(token_stream.source_id(), marker_span)),
            ),
        )
        .into());
    }

    let body_start_index = find_template_body_start(token_stream)?;
    let start_index = token_stream.position();
    // Canonical suffix window first; compatibility streams return `Ok(None)` and use the
    // explicit vector fallback below. The boundary scan stays parent-bounded because it must
    // observe the `StartTemplateBody` terminator at `body_start_index`, while every interior
    // suffix read below stays inside `[start_index, body_start_index)` via the window.
    // Dense segmented coordinates are window-relative; canonical `SourceTokens` are never
    // cloned into a new `FileTokens` vector here.
    let suffix_window = token_stream
        .subcursor_window(start_index, body_start_index)
        .map_err(TemplateError::from)?;
    let mut warnings = Vec::new();
    let (parsed_header, body_context) = match suffix_window {
        Some(mut window) => {
            if has_top_level_suffix_separator_in_window(&mut window) {
                return Err(with_token_span(
                    token_stream,
                    marker_span,
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
                        Some(SourceSpan::new(token_stream.source_id(), marker_span)),
                    ),
                )
                .into());
            }
            let eof_span = window
                .span_at(body_start_index.saturating_sub(1))
                .unwrap_or_else(|| SourceSpan::new(token_stream.source_id(), marker_span));
            let mut window = window.with_synthetic_eof_span(eof_span);
            parse_loop_header_cursor(
                &mut window,
                context.new_child_control_flow(ContextKind::Loop, string_table, path_fork),
                type_interner,
                &mut warnings,
                string_table,
                path_fork,
            )?
        }
        None => {
            let tokens: Vec<Token> = (start_index..body_start_index)
                .filter_map(|index| token_stream.token_at(index))
                .collect();
            if has_top_level_suffix_separator(&tokens) {
                return Err(with_token_span(
                    token_stream,
                    marker_span,
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
                        Some(SourceSpan::new(token_stream.source_id(), marker_span)),
                    ),
                )
                .into());
            }
            let path_syntax = token_stream
                .path_syntax_for_substream()
                .map_err(TemplateError::from)?;
            parse_loop_header_tokens(
                &tokens,
                &path_syntax,
                context.new_child_control_flow(ContextKind::Loop, string_table, path_fork),
                type_interner,
                &mut warnings,
                string_table,
                path_fork,
            )?
        }
    };

    for warning in warnings {
        context.emit_warning(warning);
    }

    let header = match parsed_header {
        ParsedLoopHeader::Conditional { mut condition } => {
            if validation_mode == TemplateControlFlowValidationMode::ConstRequired {
                condition = inline_source_consts_for_const_required_expression(
                    condition,
                    context,
                    string_table,
                );
            }

            TemplateLoopHeader::Conditional {
                condition: Box::new(condition),
            }
        }

        ParsedLoopHeader::Range { bindings, range } => TemplateLoopHeader::Range {
            bindings: Box::new(bindings),
            range: Box::new(range),
        },
        ParsedLoopHeader::Collection { bindings, iterable } => TemplateLoopHeader::Collection {
            bindings: Box::new(bindings),
            iterable: Box::new(iterable),
        },
    };

    token_stream
        .set_position(body_start_index + 1)
        .map_err(TemplateError::from)?;

    Ok(TemplateBodyParseMode::Loop(Box::new(
        TemplateLoopBodyParseInput {
            header,
            body_context,
            span: Some(SourceSpan::new(token_stream.source_id(), marker_span)),
        },
    )))
}

/// Walk past newlines to the next meaningful token.
///
/// WHAT: reports whether that token ends the suffix instead of opening a header.
/// WHY: the suffix header must reject `loop` with nothing between it and the body.
fn next_meaningful_token_is_body_boundary(token_stream: &mut AstCursor) -> bool {
    let resume = token_stream.position();
    let end = token_stream.length();
    let mut boundary = true;
    while token_stream.position() < end && !token_stream.is_at_end() {
        // A malformed payload has no `TokenKind`; the indexed scan reported a boundary there.
        if token_stream
            .current()
            .is_some_and(|current| current.to_token_kind().is_err())
        {
            break;
        }
        let kind = token_stream.current_token_kind();
        if matches!(
            kind,
            TokenKind::StartTemplateBody | TokenKind::TemplateClose | TokenKind::Eof
        ) {
            break;
        }
        if !matches!(kind, TokenKind::Newline) {
            boundary = false;
            break;
        }
        token_stream.advance();
    }
    token_stream
        .set_position(resume)
        .expect("suffix boundary resume stays inside the active parser view");
    boundary
}

fn ensure_suffix_ends_at_body_start(token_stream: &AstCursor) -> ControlFlowSuffixResult<()> {
    match token_stream.current_token_kind() {
        TokenKind::StartTemplateBody => Ok(()),
        TokenKind::Comma => Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
                None,
            ),
        )
        .into()),
        _ => Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix,
                None,
            ),
        )
        .into()),
    }
}

/// Walk to the terminator that ends a control-flow suffix.
///
/// WHAT: reports the top-level `StartTemplateBody` index, or the ready diagnostic when the
/// suffix reaches a `TemplateClose` or `Eof` first.
fn find_template_body_start(token_stream: &mut AstCursor) -> ControlFlowSuffixResult<usize> {
    let resume = token_stream.position();
    let end = token_stream.length();
    let mut nesting_depth = NestingDepth::default();
    let mut outcome: Option<ControlFlowSuffixResult<usize>> = None;
    while token_stream.position() < end && !token_stream.is_at_end() {
        // A malformed payload has no `TokenKind`; the indexed scan truncated here, so fall
        // through to the generic diagnostic below. Compatibility cursors have no `TokenRef`
        // and skip this check, reading their materialised kinds below.
        if let Some(current) = token_stream.current()
            && current.to_token_kind().is_err()
        {
            break;
        }
        let kind = token_stream.current_token_kind();
        let is_top_level = nesting_depth.is_top_level();
        if is_top_level && matches!(kind, TokenKind::StartTemplateBody) {
            outcome = Some(Ok(token_stream.position()));
            break;
        }
        if is_top_level && matches!(kind, TokenKind::TemplateClose | TokenKind::Eof) {
            let span = token_stream.current_span();
            outcome = Some(Err(with_token_span(
                token_stream,
                span.local(),
                CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix,
                    Some(span),
                ),
            )
            .into()));
            break;
        }
        // A nested `Eof` never advances on the compatibility lane, and nothing follows it on
        // the canonical lane; stop with the same truncated fallthrough the indexed scan took.
        if matches!(kind, TokenKind::Eof) {
            break;
        }
        nesting_depth.step(kind);
        token_stream.advance();
    }
    token_stream
        .set_position(resume)
        .map_err(TemplateError::from)?;
    outcome.unwrap_or_else(|| {
        Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix,
                None,
            ),
        )
        .into())
    })
}

/// Attach the exact source span for a direct suffix diagnostic.
fn with_current_token_span(
    token_stream: &AstCursor,
    diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        let mut diagnostic = diagnostic;
        diagnostic.primary_span = Some(token_stream.current_span());
        return diagnostic;
    }
    diagnostic
}

fn with_token_span(
    token_stream: &AstCursor,
    span: LocalSpan,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = Some(SourceSpan::new(token_stream.source_id(), span));
    }
    diagnostic
}

/// Read-only current-token local span on the canonical cursor view.
///
/// WHAT: reports the `LocalSpan` of the suffix marker token without advancing
/// the stream.
/// WHY: marker payloads join with `source_id` at the diagnostic site.
fn current_token_local_span(token_stream: &AstCursor) -> LocalSpan {
    token_stream.current_span().local()
}

/// Pure top-level comma scan for the compatibility loop-suffix fallback.
///
/// WHAT: reports whether the materialised suffix tokens contain a top-level `,` without touching
/// the stream.
/// WHY: compatibility streams have no canonical owner for a bounded cursor, so this explicit
/// vector lane checks the suffix before handing it to `parse_loop_header_tokens`.
fn has_top_level_suffix_separator(tokens: &[Token]) -> bool {
    let mut nesting_depth = NestingDepth::default();
    let mut pipe_depth = 0usize;

    for token in tokens {
        if nesting_depth.is_top_level() {
            if matches!(token.kind, TokenKind::TypeParameterBracket) {
                pipe_depth = if pipe_depth == 0 { 1 } else { 0 };
            } else if pipe_depth == 0 && matches!(token.kind, TokenKind::Comma) {
                return true;
            }
        }

        nesting_depth.step(&token.kind);
    }

    false
}

/// Walk the canonical suffix window looking for a top-level `,`.
///
/// WHAT: reports whether the suffix window holds a top-level comma outside `|...|` bindings.
/// WHY: the window enforces the half-open bound (dense segmented coordinates included), so the
/// scan cannot observe tokens after the `StartTemplateBody` terminator.
fn has_top_level_suffix_separator_in_window(window: &mut AstCursor) -> bool {
    let resume = window.position();
    let end = window.length();
    let mut nesting_depth = NestingDepth::default();
    let mut pipe_depth = 0usize;
    let mut separated = false;
    while window.position() < end && !window.is_at_end() {
        // A malformed payload has no `TokenKind`. Skip it, as the legacy `filter_map` slice did.
        if window
            .current()
            .is_some_and(|current| current.to_token_kind().is_err())
        {
            window.advance();
            continue;
        }
        let kind = window.current_token_kind();
        if nesting_depth.is_top_level() {
            if matches!(kind, TokenKind::TypeParameterBracket) {
                pipe_depth = if pipe_depth == 0 { 1 } else { 0 };
            } else if pipe_depth == 0 && matches!(kind, TokenKind::Comma) {
                separated = true;
                break;
            }
        }
        // `Eof` is stable under `advance` on both lanes, so stop before re-reading it.
        if matches!(kind, TokenKind::Eof) {
            break;
        }
        nesting_depth.step(kind);
        window.advance();
    }
    window
        .set_position(resume)
        .expect("suffix separator resume stays inside the active parser view");
    separated
}
