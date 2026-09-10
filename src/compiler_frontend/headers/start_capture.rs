//! Implicit entry-start body capture.
//!
//! WHAT: collects non-header top-level tokens into the active module root's implicit `start` body.
//! WHY: only the active root executes top-level runtime code; ordinary source executable code must
//! be rejected before AST lowering, while imported-root tokens are discarded after balancing.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token};

pub(crate) fn push_runtime_template_tokens_to_start_function(
    opening_template_token: Token,
    token_stream: &mut FileTokens,
    start_function_body: &mut Vec<Token>,
    string_table: &mut StringTable,
) -> Result<(), CompilerDiagnostic> {
    start_function_body.push(opening_template_token);

    // Mutation: EOF diagnostics for unclosed templates intern the expected closing delimiter
    // ("]") so the diagnostic payload can be remapped and rendered later.
    let closing_bracket = string_table.intern("]");
    crate::compiler_frontend::utilities::token_scan::consume_balanced_template_region(
        token_stream,
        |token, _token_kind| {
            start_function_body.push(token);
        },
        |location| {
            CompilerDiagnostic::unexpected_end_of_file(Some(closing_bracket), Some(location))
        },
    )
}
