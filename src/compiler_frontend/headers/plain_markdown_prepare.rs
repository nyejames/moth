//! Plain Markdown header preparation.
//!
//! WHAT: turns raw `.md` source into a private synthetic `content #String` declaration.
//! WHY: later frontend stages should see an ordinary folded constant, not Markdown-specific AST,
//!      HIR, borrow-checker, or backend paths.
//! MUST NOT: tokenize Markdown, inspect it as Moth syntax, scan rendered HTML for dependencies or
//!           symbols, or produce runtime fragments.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::headers::synthetic_content_header::{
    SyntheticContentHeaderInput, synthetic_content_header,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareOutput, FileRole, PreparedFilePathSyntax, SyntheticContentPayload,
};
use crate::compiler_frontend::plain_markdown::render_plain_markdown;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenRange;
use std::path::PathBuf;

/// Inputs needed to prepare one plain Markdown source file.
///
/// WHY: grouping these fields keeps the per-file preparation API explicit and avoids threading a
///      long argument list through pipeline branches.
pub(crate) struct PlainMarkdownPrepareInput<'a> {
    pub(crate) source_code: &'a str,
    pub(crate) source_file: PathId,
    pub(crate) file_id: SourceId,
    pub(crate) canonical_os_path: Option<PathBuf>,
}

/// Prepare one `.md` source file as a generated `content #String` constant.
///
/// WHAT: renders raw Markdown to HTML, interns the result as a compact synthetic payload, and
///       leaves the prepared source on the no-token path.
/// WHY: the rest of the frontend pipeline should not know that this constant came from Markdown.
pub(crate) fn prepare_plain_markdown_file(
    input: PlainMarkdownPrepareInput<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FileFrontendPrepareOutput, CompilerError> {
    let canonical_os_path = input.canonical_os_path.clone();
    let rendered = render_plain_markdown(input.source_code);
    let rendered_html_id = string_table.intern(&rendered.html);

    let initializer_range =
        TokenRange::from_raw(input.file_id, 0, 0).expect("empty synthetic header range is valid");
    let content_header = synthetic_content_header(
        SyntheticContentHeaderInput {
            source_file: input.source_file,
            file_id: input.file_id,
            initializer_range,
            payload: SyntheticContentPayload::RenderedHtml(rendered_html_id),
            initializer_references: Vec::new(),
        },
        string_table,
        path_fork,
    )?;

    Ok(FileFrontendPrepareOutput {
        source_file: input.source_file,
        file_id: input.file_id,
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: Vec::new(),
        structural_file_references: Default::default(),
        dependency_selections: Vec::new(),
        canonical_os_path,
        headers: vec![content_header],
        top_level_const_fragments: Vec::new(),
        source_token_stream: None,
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
#[path = "tests/plain_markdown_prepare_tests.rs"]
mod plain_markdown_prepare_tests;
