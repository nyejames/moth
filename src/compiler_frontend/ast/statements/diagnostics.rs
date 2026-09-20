//! Statement-position diagnostic helpers.
//!
//! WHAT: builds typed `CompilerDiagnostic` values for unexpected tokens and scope
//!       closes encountered during function-body statement dispatch.
//! WHY: keeps `body_dispatch.rs` focused on parsing logic rather than diagnostic
//!      construction, and ensures all statement-position errors emit structured
//!      `CompilerDiagnostic` records instead of legacy `CompilerError`.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidStatementPositionReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Attach the exact authored range of the current token when this stream has a source identity.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = Some(token_stream.current_span());
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
) -> Result<CompilerDiagnostic, CompilerError> {
    let current_tag = token_stream.current_tag();
    let span = Some(token_stream.current_span());
    let diagnostic = match current_tag {
        TokenTag::COMMA => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedComma,
            span,
        ),

        TokenTag::CLOSE_PARENTHESIS => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseParenthesis,
            span,
        ),

        TokenTag::CLOSE_CURLY => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseCurly,
            span,
        ),

        // The `|` token is only valid in type-parameter position, not as a statement.
        TokenTag::TYPE_PARAMETER_BRACKET => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedPipe,
            span,
        ),

        TokenTag::ARROW => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedArrow,
            span,
        ),

        TokenTag::WILDCARD => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedWildcard,
            span,
        ),

        // `type` in statement position looks like an attempt to declare a generic parameter.
        TokenTag::TYPE => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            span,
        ),

        TokenTag::OF => CompilerDiagnostic::invalid_statement_position(
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
                        crate::compiler_frontend::compiler_messages::CompilerDiagnostic::token_view_invariant_error(
                            error,
                            "statement-position unexpected-token diagnostic",
                        )
                    })?
                    .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
                CompilerDiagnostic::unexpected_token_from_tag(found, span)
            }
        }

        _ => {
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    crate::compiler_frontend::compiler_messages::CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "statement-position unexpected-token diagnostic",
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            CompilerDiagnostic::unexpected_token_from_tag(found, span)
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
) -> CompilerDiagnostic {
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
        CompilerDiagnostic::invalid_statement_position(reason, Some(token_stream.current_span())),
    )
}

#[cfg(test)]
#[path = "tests/diagnostics_tests.rs"]
mod diagnostics_tests;
