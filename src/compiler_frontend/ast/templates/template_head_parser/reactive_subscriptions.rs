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
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateConstructionContext;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Typed result for reactive subscription parsing.
type ReactiveSubscriptionResult<T> = Result<T, TemplateError>;

/// Parses and validates a `$(source)` template subscription.
///
/// The token stream enters on the reactive tag and exits on the token after
/// the closing `)`.
pub(super) fn parse_reactive_subscription(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    construction_context: &mut TemplateConstructionContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ReactiveSubscriptionResult<()> {
    let subscription_token_span = current_token_local_span(token_stream);
    let subscription_span = Some(SourceSpan::new(
        token_stream.source_id(),
        subscription_token_span,
    ));

    token_stream.advance();
    if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
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
    let source_token = token_stream.current();
    let source_name = match token_stream.current_tag() {
        TokenTag::CLOSE_PARENTHESIS => {
            return Err(with_current_token_span(
                token_stream,
                CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::ReactiveSubscriptionEmpty,
                    None,
                ),
            )
            .into());
        }

        TokenTag::SYMBOL => token_stream
            .current_string_id_in(string_table)?
            .ok_or_else(|| {
                CompilerError::compiler_error("reactive source symbol had no payload")
            })?,

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

    let source_token_span = current_token_local_span(token_stream);
    let source_span = Some(SourceSpan::new(token_stream.source_id(), source_token_span));

    token_stream.advance();
    if token_stream.current_tag() != TokenTag::CLOSE_PARENTHESIS {
        let reason = if token_stream.current_tag() == TokenTag::COMMA {
            InvalidTemplateStructureReason::ReactiveSubscriptionMultipleSources
        } else {
            InvalidTemplateStructureReason::ReactiveSubscriptionComplexExpression
        };

        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::invalid_template_structure(reason, None),
        )
        .into());
    }

    let Some(reference) = context.get_reference(&source_name) else {
        let found = match source_token {
            Some(source_token) => DiagnosticToken::try_from_token_ref(source_token)
                .map_err(|error| {
                    CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "reactive subscription source diagnostic",
                    )
                })
                .map_err(TemplateError::from)?,
            None => DiagnosticToken::from_static_tag(TokenTag::SYMBOL),
        };
        return Err(with_token_span(
            token_stream,
            source_token_span,
            CompilerDiagnostic::unexpected_token_from_tag(found, source_span),
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
    token_stream: &AstCursor,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = Some(token_stream.current_span());
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
/// WHAT: reports the `LocalSpan` of the reactive head token without advancing
/// the stream.
/// WHY: subscription payloads join with `source_id` at the diagnostic site.
fn current_token_local_span(token_stream: &AstCursor) -> LocalSpan {
    token_stream.current_span().local()
}
