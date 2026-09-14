//! Implicit entry-start body capture.
//!
//! WHAT: records source-owned token ranges for non-header top-level tokens and runtime templates.
//! WHY: only the active root executes top-level runtime code; ordinary source executable code must
//! be rejected before AST lowering, while imported-root tokens are discarded after balancing.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::types::HeaderParseFailure;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenIndex, TokenRange};

/// Consume one runtime template from the canonical lexer stream and return its source range.
///
/// The opening token has already been identified by the caller; `opening_index` points to that
/// token and the stream is positioned immediately after it. No token vector is retained.
pub(super) fn capture_runtime_template_range(
    opening_index: usize,
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
) -> Result<TokenRange, HeaderParseFailure> {
    let start = TokenIndex::try_from_index(opening_index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template opening exceeded the source token index space",
        ))
    })?;

    // Mutation: EOF diagnostics for unclosed templates intern the expected closing delimiter
    // ("]") so the diagnostic payload can be remapped and rendered later.
    let closing_bracket = string_table.intern("]");
    crate::compiler_frontend::utilities::token_scan::consume_balanced_template_region(
        token_stream,
        |_token, _token_kind| {},
        |location| CompilerDiagnostic::unexpected_end_of_file(Some(closing_bracket), Some(location)),
    )?;

    let end = TokenIndex::try_from_index(token_stream.index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template end exceeded the source token index space",
        ))
    })?;
    TokenRange::new(token_stream.file_id, start, end).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template source range was reversed",
        ))
    })
}
