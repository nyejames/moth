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
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokenBuildError, SourceTokens, SourceTokensBuilder, TokenIndex, TokenKind, TokenRange,
};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use std::collections::HashSet;
use std::sync::Arc;

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
/// their body as the header's checked range, while payload-only adapters are materialised by AST
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
    // Synthetic bodies are represented by `Header::tokens`; AST constant resolution builds a
    // transient canonical owner from that range and payload. Keeping the declaration shell
    // range empty prevents a second retained initializer authority.
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

/// Materialise one synthetic initializer only while AST folds its generated constant.
///
/// WHAT: builds a transient canonical owner for the generated `content #String` initializer.
/// Markdown is one interned string token. A Moth template copies its retained body window and
/// wraps it in the same `$md` template tokens the parser already understands.
/// WHY: the fold must parse through the ordinary declaration cursor without a `FileTokens`
/// vector and without retaining a second source store on the prepared source.
pub(crate) fn materialize_synthetic_content_initializer(
    payload: SyntheticContentPayload,
    source: Option<&SourceTokens>,
    body_range: TokenRange,
) -> Result<SourceTokens, CompilerError> {
    match payload {
        SyntheticContentPayload::RenderedHtml(rendered_html) => {
            rendered_html_owner(body_range.source(), rendered_html)
        }
        SyntheticContentPayload::MothTemplate { markdown_directive } => {
            let source = source.ok_or_else(|| {
                CompilerError::compiler_error(
                    "MothTemplate synthetic content has no canonical source token owner",
                )
            })?;
            moth_template_wrapper_owner(source, body_range, markdown_directive)
        }
    }
}

fn rendered_html_owner(
    source: SourceId,
    rendered_html: StringId,
) -> Result<SourceTokens, CompilerError> {
    let mut builder = SourceTokensBuilder::with_capacity(source, 1);
    push_synthetic_kind(
        &mut builder,
        &TokenKind::StringSliceLiteral(rendered_html),
        LocalSpan::source_start(),
    )?;
    finish_synthetic_owner(
        builder,
        NumericLiteralStore::with_source(source),
        frozen_empty_path_table(source)?,
    )
}

fn moth_template_wrapper_owner(
    source: &SourceTokens,
    body_range: TokenRange,
    markdown_directive: StringId,
) -> Result<SourceTokens, CompilerError> {
    let wrapper_count = 4;
    let mut builder = SourceTokensBuilder::with_capacity(
        source.source(),
        body_range.len() as usize + wrapper_count,
    );
    let mut numeric_literals = NumericLiteralStore::with_source(source.source());
    let wrapper_span = LocalSpan::source_start();

    push_synthetic_kind(&mut builder, &TokenKind::TemplateHead, wrapper_span)?;
    push_synthetic_kind(
        &mut builder,
        &TokenKind::StyleDirective(markdown_directive),
        wrapper_span,
    )?;
    push_synthetic_kind(&mut builder, &TokenKind::StartTemplateBody, wrapper_span)?;
    append_same_domain_window(source, body_range, &mut builder, &mut numeric_literals)?;
    push_synthetic_kind(&mut builder, &TokenKind::TemplateClose, wrapper_span)?;

    finish_synthetic_owner(builder, numeric_literals, source.path_syntax_arc()?)
}

fn append_same_domain_window(
    source: &SourceTokens,
    body_range: TokenRange,
    builder: &mut SourceTokensBuilder,
    numeric_literals: &mut NumericLiteralStore,
) -> Result<(), CompilerError> {
    source.validate_range(body_range).map_err(|error| {
        CompilerError::compiler_error(format!(
            "MothTemplate synthetic body range is outside its canonical source owner: {error:?}"
        ))
    })?;

    for index in body_range.start().index()..body_range.end().index() {
        let token_index = TokenIndex::try_from_index(index).ok_or_else(|| {
            CompilerError::compiler_error(
                "MothTemplate synthetic body index exceeds its checked u32 domain",
            )
        })?;
        let token = source.token(token_index).map_err(|error| {
            CompilerError::compiler_error(format!(
                "MothTemplate synthetic body token is outside its canonical source owner: {error:?}"
            ))
        })?;
        let mut shape = token.shape();
        if let Some(numeric) = token.numeric_literal().map_err(|error| {
            CompilerError::compiler_error(format!(
                "MothTemplate synthetic body numeric payload is malformed: {error:?}"
            ))
        })? {
            shape.data = numeric_literals
                .try_push_for_source(source.source(), numeric.clone())
                .map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "MothTemplate synthetic body numeric row could not be staged: {error:?}"
                    ))
                })?
                .raw();
        }
        builder
            .push_rebased_shape(shape, token.span())
            .map_err(map_synthetic_build_error)?;
    }

    Ok(())
}

fn push_synthetic_kind(
    builder: &mut SourceTokensBuilder,
    kind: &TokenKind,
    span: LocalSpan,
) -> Result<(), CompilerError> {
    builder.push(kind, span).map_err(map_synthetic_build_error)
}

fn finish_synthetic_owner(
    builder: SourceTokensBuilder,
    numeric_literals: NumericLiteralStore,
    path_syntax: Arc<PathSyntaxTable>,
) -> Result<SourceTokens, CompilerError> {
    let mut owner = builder.finish(numeric_literals)?;
    owner.attach_shared_path_syntax(path_syntax);
    owner.freeze_numeric_literals();
    Ok(owner)
}

fn frozen_empty_path_table(source: SourceId) -> Result<Arc<PathSyntaxTable>, CompilerError> {
    let mut table = PathSyntaxTable::with_source(source);
    table.validate_file_owned_locations(source)?;
    table.freeze();
    Ok(Arc::new(table))
}

fn map_synthetic_build_error(error: SourceTokenBuildError) -> CompilerError {
    match error {
        SourceTokenBuildError::Capacity => CompilerError::compiler_error(
            "synthetic content initializer exceeds its checked u32 index domain",
        ),
        SourceTokenBuildError::Invariant(error) => error,
    }
}

#[cfg(test)]
#[path = "tests/synthetic_content_header_tests.rs"]
mod synthetic_content_header_tests;
