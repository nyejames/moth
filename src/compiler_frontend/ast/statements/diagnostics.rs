//! Statement-position diagnostic helpers.
//!
//! WHAT: builds diagnostic drafts for unexpected tokens and scope closes encountered during
//!       function-body statement dispatch.
//! WHY: keeps `body_dispatch.rs` focused on parsing logic rather than diagnostic construction.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, token_lexical,
};
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidStatementPositionReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Attach the exact authored range of the current token when this stream has a source identity.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: DiagnosticDraft,
) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        diagnostic.set_primary_span(Some(token_stream.current_span()));
    }

    diagnostic
}

/// Produce a diagnostic for an unexpected token in statement position.
///
/// WHAT: maps each unexpected token tag to the most appropriate typed diagnostic.
/// WHY: centralizes the constructor decision so the statement dispatch loop stays readable.
pub(crate) fn unexpected_statement_token(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> Result<DiagnosticDraft, InfrastructureFailure> {
    let current_tag = token_stream.current_tag();
    let span = Some(token_stream.current_span());
    let diagnostic = match current_tag {
        TokenTag::COMMA => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedComma,
            span,
        ),

        TokenTag::CLOSE_PARENTHESIS => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseParenthesis,
            span,
        ),

        TokenTag::CLOSE_CURLY => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseCurly,
            span,
        ),

        // The `|` token is only valid in type-parameter position, not as a statement.
        TokenTag::TYPE_PARAMETER_BRACKET => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedPipe,
            span,
        ),

        TokenTag::ARROW => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedArrow,
            span,
        ),

        TokenTag::WILDCARD => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedWildcard,
            span,
        ),

        // `type` in statement position looks like an attempt to declare a generic parameter.
        TokenTag::TYPE => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            span,
        ),

        TokenTag::OF => choices_control_flow::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedOf,
            span,
        ),

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            if let Some(keyword) = reserved_trait_keyword_for_tag(current_tag) {
                reserved_trait_keyword_error(keyword, span)
            } else {
                // Invariant: Must and TraitThis are always reserved trait keywords.
                let found = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        InfrastructureFailure::malformed_token_projection(
                            "statement-position unexpected-token diagnostic",
                            error,
                        )
                    })?
                    .unwrap_or_else(|| {
                        DiagnosticToken::from_static_tag(token_stream.current_tag())
                    });
                token_lexical::unexpected_token_from_tag(found, span)
            }
        }

        _ => {
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    InfrastructureFailure::malformed_token_projection(
                        "statement-position unexpected-token diagnostic",
                        error,
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            token_lexical::unexpected_token_from_tag(found, span)
        }
    };

    Ok(with_current_token_span(token_stream, diagnostic))
}

/// Context for an unexpected scope-close (`;`) diagnostic.
pub(crate) enum UnexpectedScopeCloseContext {
    /// The scope close appeared inside an expression, where `;` is not valid.
    Expression,

    /// The scope close appeared inside a template literal, where `;` is not valid.
    Template,
}

/// Produce a diagnostic for an unexpected scope-close (`;`) in expression or
/// template context.
///
/// WHAT: expressions and templates are not terminated with `;`, so encountering
///       `End` inside them needs a targeted explanation.
pub(crate) fn unexpected_scope_close(
    context: UnexpectedScopeCloseContext,
    token_stream: &AstCursor,
) -> DiagnosticDraft {
    let reason = match context {
        UnexpectedScopeCloseContext::Expression => {
            InvalidStatementPositionReason::UnexpectedScopeCloseInExpression
        }
        UnexpectedScopeCloseContext::Template => {
            InvalidStatementPositionReason::UnexpectedScopeCloseInTemplate
        }
    };

    with_current_token_span(
        token_stream,
        choices_control_flow::invalid_statement_position(reason, Some(token_stream.current_span())),
    )
}

#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod diagnostics_tests;
