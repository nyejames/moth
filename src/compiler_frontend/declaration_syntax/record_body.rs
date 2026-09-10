//! Shared record-body parsing for declaration shells.
//!
//! WHAT: parses `| field Type [= default], ... |` bodies used by structs and choice payloads.
//! WHY: record bodies are a neutral declaration syntax concept, not struct-specific logic.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, SignatureMemberSyntax, parse_signature_members_syntax,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::ExtendedSpanBuilder;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

/// Two-lane result for record-body parsing.
///
/// WHAT: carries authored-source diagnostics separately from internal compiler-state failures.
/// WHY: record-body parsing otherwise carries the large diagnostic value
///      through every successful header parse. Each caller propagates both lanes.
type RecordBodyParseResult = Result<Vec<SignatureMemberSyntax>, HeaderParseFailure>;

pub fn parse_record_body(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
    member_context: SignatureMemberContext,
    owner_path: &InternedPath,
    span_builder: &mut ExtendedSpanBuilder,
) -> RecordBodyParseResult {
    token_stream.advance();
    let fields = parse_signature_members_syntax(
        token_stream,
        string_table,
        warnings,
        member_context,
        owner_path,
        span_builder,
    )?;

    token_stream.advance();

    Ok(fields)
}
