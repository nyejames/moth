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
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, Token, TokenKind, TokenRange};
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
/// their body as the header's checked range, while payload-only adapters are materialized by AST
/// only at the fold boundary.
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
    // Synthetic bodies are represented by `Header::tokens`; AST constant resolution supplies
    // the parser-only initializer override from that range and payload. Keeping the declaration
    // shell range empty prevents a second retained initializer authority.
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

/// Materialize one synthetic initializer only while AST folds its generated constant.
///
/// Moth template body tokens come from a checked range over the canonical source owner. The
/// wrapper is parser compatibility data and is therefore dropped with the returned vector rather
/// than retained on the header or prepared source.
pub(crate) fn materialize_synthetic_content_initializer(
    payload: SyntheticContentPayload,
    source: Option<&SourceTokens>,
    body_range: TokenRange,
    declaration_path: PathId,
    canonical_os_path: Option<std::path::PathBuf>,
) -> Result<Vec<Token>, CompilerError> {
    match payload {
        SyntheticContentPayload::RenderedHtml(rendered_html) => Ok(vec![
            Token::new(
                TokenKind::StringSliceLiteral(rendered_html),
                LocalSpan::source_start(),
            ),
            Token::new(TokenKind::Eof, LocalSpan::source_start()),
        ]),
        SyntheticContentPayload::MothTemplate { markdown_directive } => {
            let source = source.ok_or_else(|| {
                CompilerError::compiler_error(
                    "MothTemplate synthetic content has no canonical source token owner",
                )
            })?;
            let body_tokens = source.materialize_range(body_range)?;
            let mut initializer_tokens = Vec::with_capacity(body_tokens.len() + 5);
            initializer_tokens.push(Token::new(
                TokenKind::TemplateHead,
                LocalSpan::source_start(),
            ));
            initializer_tokens.push(Token::new(
                TokenKind::StyleDirective(markdown_directive),
                LocalSpan::source_start(),
            ));
            initializer_tokens.push(Token::new(
                TokenKind::StartTemplateBody,
                LocalSpan::source_start(),
            ));
            initializer_tokens.extend(body_tokens);
            initializer_tokens.push(Token::new(
                TokenKind::TemplateClose,
                LocalSpan::source_start(),
            ));
            initializer_tokens.push(Token::new(TokenKind::Eof, LocalSpan::source_start()));
            // The from-canonical adapter signature needs the explicit filesystem identity plus
            // the declaration shell path; materialization keeps only the token vector.
            let _ = (declaration_path, canonical_os_path);
            Ok(initializer_tokens)
        }
    }
}
