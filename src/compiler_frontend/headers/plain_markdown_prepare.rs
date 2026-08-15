//! Plain Markdown header preparation.
//!
//! WHAT: turns raw `.md` source into a private synthetic `content #String` declaration.
//! WHY: later frontend stages should see an ordinary folded constant, not Markdown-specific AST,
//!      HIR, borrow-checker, or backend paths.
//! MUST NOT: tokenize Markdown, inspect it as Moth syntax, scan rendered HTML for dependencies or
//!           symbols, or produce runtime fragments.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::headers::synthetic_content_header::{
    SyntheticContentHeaderInput, synthetic_content_header,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareOutput, FileRole, PreparedFilePathSyntax,
};
use crate::compiler_frontend::plain_markdown::render_plain_markdown;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{SourceLocation, Token, TokenKind};
use std::path::PathBuf;

/// Inputs needed to prepare one plain Markdown source file.
///
/// WHY: grouping these fields keeps the per-file preparation API explicit and avoids threading a
///      long argument list through pipeline branches.
pub(crate) struct PlainMarkdownPrepareInput<'a> {
    pub(crate) source_code: &'a str,
    pub(crate) source_file: InternedPath,
    pub(crate) file_id: Option<FileId>,
    pub(crate) canonical_os_path: Option<PathBuf>,
}

/// Prepare one `.md` source file as a generated `content #String` constant.
///
/// WHAT: renders the raw Markdown to HTML, interns the result, and builds a single literal
///       initializer token that folds cleanly to a `#String` constant.
/// WHY: the rest of the frontend pipeline should not know that this constant came from Markdown.
pub(crate) fn prepare_plain_markdown_file(
    input: PlainMarkdownPrepareInput<'_>,
    string_table: &mut StringTable,
) -> FileFrontendPrepareOutput {
    let canonical_os_path = input.canonical_os_path.clone();
    let rendered = render_plain_markdown(input.source_code);
    let rendered_html_id = string_table.intern(&rendered.html);

    let file_start_location = SourceLocation::new(
        input.source_file.clone(),
        CharPosition::default(),
        CharPosition::default(),
    );

    // A `StringSliceLiteral` is the normal top-level string literal token. It preserves the
    // already-rendered HTML exactly and folds to `#String` through the existing AST constant
    // folder without re-serializing or escaping source text.
    let initializer_tokens = vec![Token::new(
        TokenKind::StringSliceLiteral(rendered_html_id),
        file_start_location.clone(),
    )];

    let content_header = synthetic_content_header(
        SyntheticContentHeaderInput {
            source_file: input.source_file,
            file_id: input.file_id,
            canonical_os_path: canonical_os_path.clone(),
            location: file_start_location,
            initializer_tokens,
            initializer_references: Vec::new(),
        },
        string_table,
    );

    FileFrontendPrepareOutput {
        source_file: content_header.source_file.clone(),
        file_id: input.file_id,
        path_syntax: PreparedFilePathSyntax::empty(),
        token_count: 0,
        token_stats: TokenStats::default(),
        file_role: FileRole::Normal,
        file_dependency_clauses: Vec::new(),
        dependency_selections: Vec::new(),
        canonical_os_path,
        headers: vec![content_header],
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    }
}

#[cfg(test)]
#[path = "tests/plain_markdown_prepare_tests.rs"]
mod plain_markdown_prepare_tests;
