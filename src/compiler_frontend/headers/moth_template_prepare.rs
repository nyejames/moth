//! Synthetic Moth template header preparation.
//!
//! WHAT: turns a tokenized `.mtf` body into the normal private `content #String`
//! declaration consumed by dependency sorting and AST.
//! WHY: Moth template source is authored as a template body, but later frontend stages
//! should see an ordinary constant header instead of a Moth template-specific AST path
//! or textually wrapped source.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::CharPosition;
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::synthetic_content_header::{
    SyntheticContentHeaderInput, synthetic_content_header,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareOutput, FileRole, PreparedFilePathSyntax,
};
use crate::compiler_frontend::paths::file_references::classify_prepared_file_references;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::collect_symbol_references;
use std::path::PathBuf;

const MOTH_TEMPLATE_MARKDOWN_DIRECTIVE: &str = "md";

/// Build the header-stage output for one `.mtf` source file.
///
/// The input token stream must already have been tokenized with Moth template's template-body entry
/// policy. This function adds only structural wrapper tokens around those body tokens; it never
/// prepends or appends source text.
pub(crate) fn prepare_moth_template_file(
    mut file_tokens: FileTokens,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, CompilerError> {
    let token_count = file_tokens.length;
    let token_stats = file_tokens.token_stats;
    let path_syntax = PreparedFilePathSyntax::from_file_tokens(&mut file_tokens)?;
    let structural_file_references = classify_prepared_file_references(
        path_syntax.table(),
        [],
        file_tokens.file_id,
        string_table,
    );
    let context = MothTemplatePrepareContext::new(file_tokens, string_table);
    let content_header = context.content_header(string_table);

    let mut headers = vec![content_header];

    // Content sources can reference other content sources, so the synthetic constant's template
    // body takes the same token-level content ordering facts as authored shells.
    collect_content_source_ordering_hints(&mut headers, &structural_file_references, string_table);

    Ok(FileFrontendPrepareOutput {
        source_file: context.source_file,
        file_id: context.file_id,
        path_syntax,
        token_count,
        token_stats,
        file_role: FileRole::Normal,
        file_dependency_clauses: Vec::new(),
        structural_file_references,
        dependency_selections: Vec::new(),
        canonical_os_path: context.canonical_os_path,
        headers,
        top_level_const_fragments: Vec::new(),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    })
}

/// File-local data needed to synthesize the normal constant header.
///
/// Keeping these fields together makes the generated token construction explicit without
/// threading the same path, location, and interned names through every helper.
struct MothTemplatePrepareContext {
    source_file: InternedPath,
    file_id: Option<FileId>,
    canonical_os_path: Option<PathBuf>,
    body_tokens: Vec<Token>,
    synthetic_location: SourceLocation,
    markdown_directive: StringId,
}

impl MothTemplatePrepareContext {
    fn new(file_tokens: FileTokens, string_table: &mut StringTable) -> Self {
        let synthetic_location = SourceLocation::new(
            file_tokens.src_path.clone(),
            CharPosition::default(),
            CharPosition::default(),
        );
        let markdown_directive = string_table.intern(MOTH_TEMPLATE_MARKDOWN_DIRECTIVE);

        let body_tokens = file_tokens
            .tokens
            .into_iter()
            .filter(|token| !matches!(token.kind, TokenKind::ModuleStart | TokenKind::Eof))
            .collect();

        Self {
            source_file: file_tokens.src_path,
            file_id: file_tokens.file_id,
            canonical_os_path: file_tokens.canonical_os_path,
            body_tokens,
            synthetic_location,
            markdown_directive,
        }
    }

    fn content_header(
        &self,
        string_table: &mut StringTable,
    ) -> crate::compiler_frontend::headers::types::Header {
        let initializer_tokens = self.template_initializer_tokens();
        let initializer_references = collect_symbol_references(&initializer_tokens);

        synthetic_content_header(
            SyntheticContentHeaderInput {
                source_file: self.source_file.clone(),
                file_id: self.file_id,
                canonical_os_path: self.canonical_os_path.clone(),
                location: self.synthetic_location.clone(),
                initializer_tokens,
                initializer_references,
            },
            string_table,
        )
    }

    fn template_initializer_tokens(&self) -> Vec<Token> {
        let mut initializer_tokens = Vec::with_capacity(self.body_tokens.len() + 4);

        initializer_tokens.push(self.synthetic_token(TokenKind::TemplateHead));
        initializer_tokens
            .push(self.synthetic_token(TokenKind::StyleDirective(self.markdown_directive)));
        initializer_tokens.push(self.synthetic_token(TokenKind::StartTemplateBody));
        initializer_tokens.extend(self.body_tokens.iter().cloned());
        initializer_tokens.push(self.synthetic_token(TokenKind::TemplateClose));

        initializer_tokens
    }

    fn synthetic_token(&self, kind: TokenKind) -> Token {
        Token::new(kind, self.synthetic_location.clone())
    }
}

#[cfg(test)]
#[path = "tests/moth_template_prepare_tests.rs"]
mod moth_template_prepare_tests;
