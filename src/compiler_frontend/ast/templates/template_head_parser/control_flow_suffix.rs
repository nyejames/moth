//! Template head suffix parsing for `if` and `loop`.
//!
//! WHAT: turns final template-head control-flow suffixes into a structured body
//! parser mode.
//! WHY: the head parser must recognize control flow before body parsing, but
//! branch/body splitting belongs to the body parser in the next phase.

use crate::compiler_frontend::ast::statements::if_headers::{ParsedIfHeader, parse_if_header};
use crate::compiler_frontend::ast::statements::loop_headers::{
    ParsedLoopHeader, parse_loop_header_tokens,
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
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

/// Template head control flow joins ordinary expression parsing with template construction. It
/// preserves both authored diagnostics and retained-data infrastructure failures until the
/// template's owning construction boundary.
type ControlFlowSuffixResult<T> = Result<T, TemplateError>;

/// Parse a template `if` suffix after the `if` token has been seen.
pub(crate) fn parse_if_suffix(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    validation_mode: TemplateControlFlowValidationMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ControlFlowSuffixResult<TemplateBodyParseMode> {
    // Short-lived canonical view for this token-local marker span; dropped
    // before the grammar advance and recursive `parse_if_header` handoff.
    // `current_token` stays the documented `FileTokens` grammar boundary
    // (fallback below).
    let marker_span = current_token_local_span(token_stream);
    token_stream.advance(); // consume `if`

    if next_meaningful_token_is_body_boundary(token_stream) {
        return Err(with_token_span(
            token_stream,
            marker_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MissingTemplateIfCondition,
                Some(SourceSpan::new(token_stream.file_id, marker_span)),
            ),
        )
        .into());
    }

    let parsed_header =
        parse_if_header(token_stream, context, type_interner, string_table, path_fork)?;

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

    let else_context =
        context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);

    Ok(TemplateBodyParseMode::If(Box::new(
        TemplateIfBodyParseInput {
            selector: condition,
            then_context,
            else_context,
            span: Some(SourceSpan::new(token_stream.file_id, marker_span)),
        },
    )))
}

/// Parse a template `loop` suffix after the `loop` token has been seen.
pub(crate) fn parse_loop_suffix(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    validation_mode: TemplateControlFlowValidationMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ControlFlowSuffixResult<TemplateBodyParseMode> {
    // Short-lived canonical view for this token-local marker span; dropped
    // before the grammar advance and recursive header scan. `current_token`
    // stays the documented `FileTokens` grammar boundary (fallback below).
    let marker_span = current_token_local_span(token_stream);
    token_stream.advance(); // consume `loop`

    if next_meaningful_token_is_body_boundary(token_stream) {
        return Err(with_token_span(
            token_stream,
            marker_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MissingTemplateLoopHeader,
                Some(SourceSpan::new(token_stream.file_id, marker_span)),
            ),
        )
        .into());
    }

    let body_start_index = find_template_body_start(token_stream)?;
    let suffix_tokens = &token_stream.tokens[token_stream.index..body_start_index];

    if has_top_level_suffix_separator(suffix_tokens) {
        return Err(with_token_span(
            token_stream,
            marker_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
                Some(SourceSpan::new(token_stream.file_id, marker_span)),
            ),
        )
        .into());
    }

    let mut warnings = Vec::new();
    let (parsed_header, body_context) = parse_loop_header_tokens(
        suffix_tokens,
        &token_stream.path_syntax,
        context.new_child_control_flow(ContextKind::Loop, string_table, path_fork),
        type_interner,
        &mut warnings,
        string_table,
        path_fork,
    )?;

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

    token_stream.index = body_start_index + 1;

    Ok(TemplateBodyParseMode::Loop(Box::new(
        TemplateLoopBodyParseInput {
            header,
            body_context,
            span: Some(SourceSpan::new(token_stream.file_id, marker_span)),
        },
    )))
}

fn next_meaningful_token_is_body_boundary(token_stream: &FileTokens) -> bool {
    // Short-lived canonical view for this pure newline-skipping lookahead;
    // dropped before any `FileTokens` mutation or recursive parse.
    // Compatibility-only streams keep the checked vector lane as the
    // documented `FileTokens` grammar boundary (fallback below).
    if let Ok(cursor) = DeclarationCursor::from_file_tokens(token_stream) {
        return next_meaningful_token_is_body_boundary_at_cursor(&cursor);
    }
    next_meaningful_token_is_body_boundary_in_tokens(token_stream)
}

/// Cursor-view form of [`next_meaningful_token_is_body_boundary`].
fn next_meaningful_token_is_body_boundary_at_cursor(cursor: &DeclarationCursor) -> bool {
    let mut index = cursor.position();

    while index < cursor.length {
        match cursor.token_kind_at(index) {
            Some(TokenKind::Newline) => index += 1,
            Some(TokenKind::StartTemplateBody | TokenKind::TemplateClose | TokenKind::Eof) => {
                return true;
            }
            Some(_) => return false,
            None => return true,
        }
    }

    true
}

fn next_meaningful_token_is_body_boundary_in_tokens(token_stream: &FileTokens) -> bool {
    let mut index = token_stream.index;

    while index < token_stream.length {
        match token_stream.tokens[index].kind {
            TokenKind::Newline => index += 1,
            TokenKind::StartTemplateBody | TokenKind::TemplateClose | TokenKind::Eof => {
                return true;
            }
            _ => return false,
        }
    }

    true
}

fn ensure_suffix_ends_at_body_start(token_stream: &FileTokens) -> ControlFlowSuffixResult<()> {
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

fn find_template_body_start(token_stream: &FileTokens) -> ControlFlowSuffixResult<usize> {
    // Short-lived canonical view for this pure body-boundary scan; dropped
    // before the mutable stream repositioning in `parse_loop_suffix`.
    // Compatibility-only streams keep the checked vector lane as the
    // documented `FileTokens` grammar boundary (fallback below). Indexes stay
    // adapter-relative in both lanes.
    if let Ok(cursor) = DeclarationCursor::from_file_tokens(token_stream) {
        // Canonical handoff failure inside the scan still surfaces the same
        // authored diagnostic; only a missing canonical provenance falls back
        // to the vector lane.
        match find_template_body_start_at_cursor(token_stream, &cursor) {
            Ok(index) => return Ok(index),
            Err(Some(error)) => return Err(error),
            Err(None) => {}
        }
    }
    find_template_body_start_in_tokens(token_stream)
}

/// Cursor-view form of [`find_template_body_start`]. `Ok` carries the
/// adapter-relative body index; `Err(Some)` carries the ready diagnostic and
/// `Err(None)` signals the caller should use the vector fallback.
fn find_template_body_start_at_cursor(
    token_stream: &FileTokens,
    cursor: &DeclarationCursor,
) -> Result<usize, Option<TemplateError>> {
    let mut nesting_depth = NestingDepth::default();
    let mut index = cursor.position();

    while index < cursor.length {
        let Some(kind) = cursor.token_kind_at(index) else {
            // Canonical lane ended before the vector lane; let the checked
            // vector fallback finish the scan on adapter-relative coordinates.
            return Err(None);
        };
        if nesting_depth.is_top_level() && matches!(kind, TokenKind::StartTemplateBody) {
            return Ok(index);
        }

        if nesting_depth.is_top_level()
            && matches!(kind, TokenKind::TemplateClose | TokenKind::Eof)
        {
            let Some(span) = cursor.span_at(index) else {
                // Canonical span unavailable; the vector fallback reports the
                // same boundary on adapter-relative coordinates.
                return Err(None);
            };
            return Err(Some(
                with_token_span(
                    token_stream,
                    span.local(),
                    CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix,
                        Some(span),
                    ),
                )
                .into(),
            ));
        }

        nesting_depth.step(&kind);
        index += 1;
    }

    Err(None)
}

fn find_template_body_start_in_tokens(
    token_stream: &FileTokens,
) -> ControlFlowSuffixResult<usize> {
    let mut nesting_depth = NestingDepth::default();
    let mut index = token_stream.index;

    while index < token_stream.length {
        let token = &token_stream.tokens[index];
        if nesting_depth.is_top_level() && matches!(token.kind, TokenKind::StartTemplateBody) {
            return Ok(index);
        }

        if nesting_depth.is_top_level()
            && matches!(token.kind, TokenKind::TemplateClose | TokenKind::Eof)
        {
            return Err(with_token_span(
                token_stream,
                token.span,
                CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix,
                    Some(SourceSpan::new(token_stream.file_id, token.span)),
                ),
            )
            .into());
        }

        nesting_depth.step(&token.kind);
        index += 1;
    }

    Err(with_current_token_span(
        token_stream,
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::UnexpectedTokenAfterControlFlowSuffix,
            None,
        ),
    )
    .into())
}

/// Attach the exact source span for a direct suffix diagnostic.
fn with_current_token_span(
    token_stream: &FileTokens,
    diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        // Short-lived canonical view for this token-local suffix span; dropped
        // before any `FileTokens` mutation. Compatibility-only streams keep the
        // checked vector lane as the documented fallback; every call site now
        // passes `None` so this cursor-first lane is live.
        let span = DeclarationCursor::from_file_tokens(token_stream)
            .ok()
            .and_then(|cursor| cursor.current_span())
            .or_else(|| {
                token_stream
                    .tokens
                    .get(token_stream.index)
                    .map(|token| SourceSpan::new(token_stream.file_id, token.span))
            });
        let mut diagnostic = diagnostic;
        diagnostic.primary_span = span;
        return diagnostic;
    }
    diagnostic
}

fn with_token_span(
    token_stream: &FileTokens,
    span: LocalSpan,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = Some(SourceSpan::new(token_stream.file_id, span));
    }
    diagnostic
}

/// Read-only current-token local span through a short canonical view.
///
/// WHAT: reports the `LocalSpan` of the suffix marker token without advancing
/// the stream.
/// WHY: marker payloads join with `file_id` at the diagnostic site; a short
/// `DeclarationCursor` keeps that fact read-only and drops before the grammar
/// advance. Unbounded compatibility-only streams fall back to the explicit
/// vector lane rather than inventing a bridge.
fn current_token_local_span(token_stream: &FileTokens) -> LocalSpan {
    if let Ok(cursor) = DeclarationCursor::from_file_tokens(token_stream) {
        if let Some(span) = cursor.current_span() {
            return span.local();
        }
    }
    token_stream
        .tokens
        .get(token_stream.index)
        .map(|token| token.span)
        .expect("control-flow suffix marker requires a current token")
}

/// Pure top-level comma scan over the already-sliced loop-suffix window.
///
/// WHAT: reports whether the suffix window contains a top-level `,` without
/// touching the stream.
/// WHY: the suffix window is a `FileTokens` compatibility slice feeding
/// `parse_loop_header_tokens`, which owns the `&[Token]` grammar boundary;
/// probing it through a short cursor would copy or reverse the adapter and
/// break that handoff. The scan stays on the slice payload while surrounding
/// lookahead/span helpers are cursor-first. `Token` payloads/spans only are
/// read — no cursor is retained.
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
