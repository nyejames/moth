//! Deferred declared-region statement header classification.
//!
//! WHAT: claims immediate `identifier:` headers in executable statement position for declared
//!       regions and rejects the exact anonymous `_:` spelling.
//! WHY: declared-region parsing and placement semantics are deferred, but this final source spelling must
//!      take precedence over existing-reference, external-call and declaration dispatch.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidStatementPositionReason,
};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

pub(crate) fn classify_deferred_declared_region_header(
    token_stream: &FileTokens,
) -> Option<CompilerDiagnostic> {
    if token_stream.peek_next_token() != Some(&TokenKind::Colon) {
        return None;
    }

    let location = token_stream.current_location();
    match token_stream.current_token_kind() {
        TokenKind::Symbol(_) => Some(CompilerDiagnostic::deferred_feature_reason(
            DeferredFeatureReason::DeclaredRegion,
            location,
        )),
        TokenKind::Wildcard => Some(CompilerDiagnostic::invalid_statement_position(
            InvalidStatementPositionReason::AnonymousDeclaredRegion,
            location,
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/declared_regions_tests.rs"]
mod declared_regions_tests;
