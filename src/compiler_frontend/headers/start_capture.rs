//! Implicit entry-start body capture.
//!
//! WHAT: records source-owned token ranges for non-header top-level tokens and runtime templates.
//! WHY: only the active root executes top-level runtime code; ordinary source executable code must
//! be rejected before AST lowering, while imported-root tokens are discarded after balancing.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::types::HeaderParseFailure;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TokenCursor, TokenRange, TokenRef, TokenTag};
use crate::compiler_frontend::utilities::token_scan::TemplateBalance;

/// Consume one runtime template directly through a borrowed canonical cursor.
///
/// The opening token has already been identified by the caller; `cursor` points immediately after
/// it. All balance, EOF and range facts come from `TokenRef`/`TokenCursor`, and no compatibility
/// token vector is materialized.
pub(super) fn capture_runtime_template_range_from_cursor(
    opening: TokenRef<'_>,
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    string_table: &mut StringTable,
) -> Result<TokenRange, HeaderParseFailure> {
    if opening.source() != file_id || opening.tag() != TokenTag::TEMPLATE_HEAD {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(
                "runtime template opening does not match its canonical header identity",
            ),
        ));
    }
    let expected_position = opening.index().index().checked_add(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
            "runtime template body position exceeded its source token index space",
        ))
    })?;
    if cursor.position().index() != expected_position {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error(
                "runtime template cursor was not positioned after its opening token",
            ),
        ));
    }

    let closing_bracket = string_table.intern("]");
    let mut balance = TemplateBalance::with_opening_template();
    while balance.has_unclosed_templates() {
        let token = cursor.advance().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "runtime template cursor reached the source end before its closing delimiter",
            ))
        })?;
        if token.is_eof() {
            return Err(CompilerDiagnostic::unexpected_end_of_file(
                Some(closing_bracket),
                Some(token.source_span()),
            )
            .into());
        }
        balance.step_tag(token.tag());
    }

    let end = cursor.position();
    TokenRange::try_new_for(cursor.source_tokens(), opening.index(), end).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "runtime template source range exceeded its canonical owner: {error:?}",
        )))
    })
}
