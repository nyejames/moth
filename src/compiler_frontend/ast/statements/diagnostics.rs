//! Statement-position diagnostic helpers.
//!
//! WHAT: builds typed `CompilerDiagnostic` values for unexpected tokens and scope
//!       closes encountered during function-body statement dispatch.
//! WHY: keeps `body_dispatch.rs` focused on parsing logic rather than diagnostic
//!      construction, and ensures all statement-position errors emit structured
//!      `CompilerDiagnostic` records instead of legacy `CompilerError`.

use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword, reserved_trait_keyword_error,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidStatementPositionReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Attach the exact authored range of the current token when this stream has a source identity.
fn with_current_token_span(
    token_stream: &FileTokens,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = Some(token_stream.current_span());
    }

    diagnostic
}

/// Produce a diagnostic for an unexpected token in statement position.
///
/// WHAT: maps each unexpected token kind to the most appropriate typed diagnostic.
/// WHY: centralizes the decision about which constructor to use so the dispatch
///      loop stays readable.
pub(crate) fn unexpected_statement_token(
    token_stream: &FileTokens,
    _string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let token_kind = token_stream.current_token_kind();
    let span = Some(token_stream.current_span());
    let diagnostic = match token_kind {
        TokenKind::Comma => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedComma,
            span,
        ),

        TokenKind::CloseParenthesis => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseParenthesis,
            span,
        ),

        TokenKind::CloseCurly => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedCloseCurly,
            span,
        ),

        // The `|` token is only valid in type-parameter position, not as a statement.
        TokenKind::TypeParameterBracket => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedPipe,
            span,
        ),

        TokenKind::Arrow => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedArrow,
            span,
        ),

        TokenKind::Wildcard => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedWildcard,
            span,
        ),

        // `type` in statement position looks like an attempt to declare a generic parameter.
        TokenKind::Type => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::GenericParameterOutsideDeclarationHeader,
            span,
        ),

        TokenKind::Of => CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::UnexpectedOf,
            span,
        ),

        TokenKind::Must | TokenKind::TraitThis => {
            if let Some(keyword) = reserved_trait_keyword(token_kind) {
                reserved_trait_keyword_error(keyword, span)
            } else {
                // Invariant: Must and TraitThis are always reserved trait keywords.
                CompilerDiagnostic::unexpected_token(token_kind.to_owned(), span)
            }
        }

        _ => CompilerDiagnostic::unexpected_token(token_kind.to_owned(), span),
    };

    with_current_token_span(token_stream, diagnostic)
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
    token_stream: &FileTokens,
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
