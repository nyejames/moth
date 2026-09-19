//! Deferred declared-region statement header classification.
//!
//! WHAT: claims immediate `identifier:` headers in executable statement position for declared
//!       regions and rejects the exact anonymous `_:` spelling.
//! WHY: declared-region parsing and placement semantics are deferred, but this final source spelling must
//!      take precedence over existing-reference, external-call and declaration dispatch.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidStatementPositionReason,
};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

pub(crate) fn classify_deferred_declared_region_header(
    token_stream: &AstCursor,
) -> Option<CompilerDiagnostic> {
    // Statement-position `identifier:` classification reads one token of tag lookahead.
    let next_is_colon = token_stream.peek_next_tag() == Some(TokenTag::COLON);
    if !next_is_colon {
        return None;
    }

    let span = Some(token_stream.current_span());
    match token_stream.current_tag() {
        TokenTag::SYMBOL => Some(CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::DeclaredRegion,
            span,
        )),
        TokenTag::WILDCARD => Some(CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::AnonymousDeclaredRegion,
            span,
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/declared_regions_tests.rs"]
mod declared_regions_tests;
