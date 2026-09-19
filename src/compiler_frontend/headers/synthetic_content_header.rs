//! Shared helper for compiler-generated `content #String` headers.
//!
//! WHAT: builds ordinary private `content #String` headers for compiler-generated source assets
//!       such as Moth template `.mtf` and plain Markdown `.md`.
//! WHY: `.mtf` retains a source-owned body range while `.md` carries rendered HTML as an explicit
//!      payload; both fold through the ordinary private `content #String` declaration.
//!      This helper owns the common header shape without duplicating source-kind storage.
//! MUST NOT: render Markdown, tokenize source, parse dependency clauses, own source-kind decisions, or
//!           construct source-location facts from filesystem paths.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::headers::types::{
    FileRole, Header, HeaderExportMode, HeaderKind, SyntheticContentPayload,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::tokenizer::tokens::TokenRange;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use std::collections::HashSet;

const SYNTHETIC_CONTENT_NAME: &str = "content";

/// Inputs needed to build one synthetic `content #String` header.
///
/// WHY: grouping these fields keeps the source-kind adapter responsible for its compact payload
/// and checked syntax range while this helper owns the repetitive header shape.
pub(crate) struct SyntheticContentHeaderInput {
    pub(crate) source_file: PathId,
    pub(crate) file_id: SourceId,
    pub(crate) initializer_range: TokenRange,
    pub(crate) payload: SyntheticContentPayload,
    pub(crate) initializer_references: Vec<InitializerReference>,
}

/// The synthetic content constant's graph path for one content source.
///
/// WHAT: derives `<content source>/content` — the exact path `synthetic_content_header` owns — so
///       ordering facts can target the generated constant without a second content name.
/// WHY: declaration ordering and the header stage must agree on the content constant identity from
///       one owner of the synthetic name.
pub(crate) fn content_constant_path(
    content_source: PathId,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<PathId, CompilerError> {
    let content_name = string_table.intern(SYNTHETIC_CONTENT_NAME);
    path_fork
        .try_intern_child(content_source, content_name)
        .ok_or_else(|| {
            CompilerError::compiler_error("path table exhausted while interning content path")
        })
}

/// Build a private `content #String` constant header from a compact adapter payload.
///
/// The declaration shell intentionally retains no initializer tokens. Tokenized adapters retain
/// their body as the header's checked range; payload-only adapters stay as a compact value and
/// become an ordinary string expression at constant resolution.
pub(crate) fn synthetic_content_header(
    input: SyntheticContentHeaderInput,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Header, CompilerError> {
    if input.initializer_range.source() != input.file_id {
        return Err(CompilerError::compiler_error(
            "synthetic content range does not match its prepared source identity",
        ));
    }
    let header_path = content_constant_path(input.source_file, path_fork, string_table)?;
    let initializer_range = None;
    let declaration = DeclarationSyntax {
        binding_mode: BindingMode::CompileTimeConstant,
        type_annotation: ParsedTypeRef::BuiltinString { span: None },
        config_qualifier: None,
        initializer_range,
        initializer_references: input.initializer_references,
        span: None,
    };

    Ok(Header {
        kind: HeaderKind::Constant { declaration },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: HashSet::new(),
        name_span: None,
        synthetic_content_payload: Some(input.payload),
        tokens: input.initializer_range,
        declaration_path: header_path,
        token_sequence: None,
        capacity_references: Vec::new(),
    })
}
