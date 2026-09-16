//! Deferred declared-region statement header classification.
//!
//! WHAT: claims immediate `identifier:` headers in executable statement position for declared
//!       regions and rejects the exact anonymous `_:` spelling.
//! WHY: declared-region parsing and placement semantics are deferred, but this final source spelling must
//!      take precedence over existing-reference, external-call and declaration dispatch.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidStatementPositionReason,
};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

pub(crate) fn classify_deferred_declared_region_header(
    token_stream: &FileTokens,
) -> Option<CompilerDiagnostic> {
    // Short-lived canonical view for this token-local colon fact; dropped before any
    // FileTokens use. Narrowed streams keep `peek_next_token` as the documented
    // FileTokens grammar boundary (fallback below).
    let next_is_colon = DeclarationCursor::from_file_tokens(token_stream)
        .map(|cursor| {
            cursor
                .position()
                .checked_add(1)
                .and_then(|next| cursor.token_kind_at(next))
                == Some(TokenKind::Colon)
        })
        .unwrap_or_else(|_| token_stream.peek_next_token() == Some(&TokenKind::Colon));
    if !next_is_colon {
        return None;
    }

    let span = Some(token_stream.current_span());
    match token_stream.current_token_kind() {
        TokenKind::Symbol(_) => Some(CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::DeclaredRegion,
            span,
        )),
        TokenKind::Wildcard => Some(CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::AnonymousDeclaredRegion,
            span,
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/declared_regions_tests.rs"]
mod declared_regions_tests;
