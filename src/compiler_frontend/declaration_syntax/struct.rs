//! Struct field-list shell parsing.
//!
//! WHAT: wraps shared record-body parsing for `Struct = | ... |` field declarations.
//! WHY: struct fields share the record-body grammar with other `| ... |` declarations. Defaults
//! are folded and checked during AST resolution, not in this shell parser.
//!
//! This module is the authoritative home for the struct shell parser. It returns neutral field
//! syntax; AST type resolution later turns that syntax into typed declarations.

use super::DeclarationCursor;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::declaration_syntax::record_body::parse_record_body;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
/// Two-lane result for struct shell parsing.
///
/// WHAT: mirrors `RecordBodyParseResult` so the thin `parse_struct_shell` wrapper
///       propagates both lanes from `parse_record_body` directly.
/// WHY: struct shell parsing is a delegation layer; the two-lane boundary belongs to
///      record-body parsing, and plain-`CompilerDiagnostic` callers extract their inline lane once.
type StructShellResult = Result<Vec<SignatureMemberSyntax>, HeaderParseFailure>;

/// Parse a struct field-list shell from `| field Type [= default], ... |` syntax.
///
/// WHAT: delegates to `parse_record_body` with the struct-field context, which advances past the
/// opening and closing `|` and parses each field into neutral syntax. Defaults stay unfolded here.
/// WHY: this is the single canonical struct field parser. Header parsing and body-local
/// declarations both use it to populate struct field lists.
pub fn parse_struct_shell(
    token_stream: &mut DeclarationCursor<'_>,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
    owner_path: PathId,
    path_fork: &mut PathInternerFork,
) -> StructShellResult {
    parse_record_body(
        token_stream,
        string_table,
        warnings,
        SignatureMemberContext::StructField,
        owner_path,
        path_fork,
    )
}
