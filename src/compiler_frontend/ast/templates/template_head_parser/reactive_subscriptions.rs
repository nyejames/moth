//! Reactive template subscription parsing.
//!
//! WHAT:
//! - Parses the V1 `$(source)` head item grammar.
//! - Resolves the source identifier through the normal scope context.
//! - Attaches subscription metadata to the parser TIR head node.
//!
//! WHY:
//! - Subscription syntax is intentionally narrower than expression interpolation. Keeping it
//!   isolated prevents general expression dependency tracking from leaking into templates.

use super::head_expressions::{
    TemplateHeadExpressionContext, push_template_head_reactive_subscription,
};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateConstructionContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Typed result for reactive subscription parsing.
type ReactiveSubscriptionResult<T> = Result<T, TemplateError>;

/// Parses and validates a `$(source)` template subscription.
///
/// The token stream enters on `TokenKind::Reactive` and exits on the token after
/// the closing `)`.
pub(super) fn parse_reactive_subscription(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    construction_context: &mut TemplateConstructionContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ReactiveSubscriptionResult<()> {
    // Short-lived canonical view for this token-local subscription span;
    // dropped before the grammar advance below. `current_token` stays the
    // documented `FileTokens` grammar boundary (fallback below).
    let subscription_token_span = current_token_local_span(token_stream);
    let subscription_span = Some(SourceSpan::new(
        token_stream.file_id,
        subscription_token_span,
    ));

    token_stream.advance();
    if token_stream.current_token_kind() != &TokenKind::OpenParenthesis {
        return Err(with_token_span(
            token_stream,
            subscription_token_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::ReactiveSubscriptionComplexExpression,
                subscription_span,
            ),
        )
        .into());
    }

    token_stream.advance();
    let source_name = match token_stream.current_token_kind() {
        TokenKind::CloseParenthesis => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::ReactiveSubscriptionEmpty,
                    None,
                ),
            )
            .into());
        }

        TokenKind::Symbol(source_name) => *source_name,

        _ => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::ReactiveSubscriptionComplexExpression,
                    None,
                ),
            )
            .into());
        }
    };

    // Short-lived canonical view for this token-local source span; dropped
    // before the grammar advance below. Fallback preserves the checked vector
    // lane for compatibility-only streams.
    let source_token_span = current_token_local_span(token_stream);
    let source_span = Some(SourceSpan::new(token_stream.file_id, source_token_span));

    token_stream.advance();
    if token_stream.current_token_kind() != &TokenKind::CloseParenthesis {
        let reason = if token_stream.current_token_kind() == &TokenKind::Comma {
            InvalidTemplateStructureReason::ReactiveSubscriptionMultipleSources
        } else {
            InvalidTemplateStructureReason::ReactiveSubscriptionComplexExpression
        };

        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_structure(
                reason,
                None,
            ),
        )
        .into());
    }

    let Some(reference) = context.get_reference(&source_name) else {
        return Err(with_token_span(
            token_stream,
            source_token_span,
            CompilerDiagnostic::unexpected_token(TokenKind::Symbol(source_name), source_span),
        )
        .into());
    };

    let Some(source) = reference.value.reactive_source.clone() else {
        return Err(with_token_span(
            token_stream,
            source_token_span,
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::ReactiveSubscriptionNonReactiveSource,
                source_span,
            ),
        )
        .into());
    };

    let expression = Expression::reference_with_type_id(
        reference.id.to_owned(),
        reference.value.diagnostic_type.to_owned(),
        reference.value.type_id,
        source_span,
        reference.value.value_mode.to_owned(),
        reference.value.const_record_state,
    )
    .with_reactive_source(source.clone());

    push_template_head_reactive_subscription(
        expression,
        source,
        TemplateHeadExpressionContext {
            context,
            type_environment,
            construction_context,
            path_fork: &*path_fork,
        },
        subscription_span,
        string_table,
    )?;

    token_stream.advance();
    Ok(())
}

/// Attach the authored span for a reactive-head syntax diagnostic.
fn with_current_token_span(
    token_stream: &FileTokens,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        // Short-lived canonical view for this token-local reactive span;
        // dropped before any `FileTokens` advance. Compatibility-only streams
        // keep the checked vector lane as the documented fallback; every call
        // site now passes `None` so this cursor-first lane is live.
        diagnostic.primary_span = DeclarationCursor::from_file_tokens(token_stream)
            .ok()
            .and_then(|cursor| cursor.current_span())
            .or_else(|| {
                token_stream
                    .tokens
                    .get(token_stream.index)
                    .map(|token| SourceSpan::new(token_stream.file_id, token.span))
            });
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
/// WHAT: reports the `LocalSpan` of the reactive head token without advancing
/// the stream.
/// WHY: subscription payloads join with `file_id` at the diagnostic site; a
/// short `DeclarationCursor` keeps that fact read-only and drops before the
/// grammar advance. Compatibility-only streams have no canonical provenance,
/// so the checked vector lane stays as the documented fallback; the current
/// token is guaranteed at every subscription read, so a missing entry is an
/// invariant failure rather than a silent span.
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
        .expect("reactive subscription span requires a current token")
}
