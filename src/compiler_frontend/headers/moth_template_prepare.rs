//! Synthetic Moth template header preparation.
//!
//! WHAT: turns a tokenized `.mtf` body into the normal private `content #String`
//! declaration consumed by dependency sorting and AST.
//! WHY: Moth template source is authored as a template body, but later frontend stages
//! should see an ordinary constant header instead of a Moth template-specific AST path
//! or textually wrapped source.
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::declaration_syntax::build_config_contract::find_config_qualifier_marker_in_cursor;
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::synthetic_content_header::{
    SyntheticContentHeaderInput, synthetic_content_header,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareOutput, FileRole, PreparedFilePathSyntax, SourceTokenOwner,
    SyntheticContentPayload,
};
use crate::compiler_frontend::paths::file_references::classify_prepared_file_references;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;

use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{SourceTokens, TokenIndex, TokenRange};
use crate::compiler_frontend::utilities::token_scan::{
    TokenFactView, collect_scanned_symbol_references,
};
use std::sync::Arc;
const MOTH_TEMPLATE_MARKDOWN_DIRECTIVE: &str = "md";

/// Build the header-stage output for one `.mtf` source file.
///
/// The canonical source owner must be paired with its preparing path table. Preparation retains
/// only a checked body range and compact directive payload; the AST fold boundary supplies the
/// structural `MothContentTemplateEntry` directly.
pub(crate) fn prepare_moth_template_file(
    owner: SourceTokenOwner,
    source_file: PathId,
    path_syntax: Arc<PathSyntaxTable>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, CompilerError> {
    let file_id = owner.source_id();
    let canonical = owner.tokens_ref();
    if canonical.source() != file_id {
        return Err(CompilerError::compiler_error(
            "Moth template source token owner does not match its file identity",
        ));
    }
    let token_count = owner.len();
    let token_stats = owner.token_stats();
    let body_range = MothTemplatePrepareContext::body_range(canonical)?;
    let context = MothTemplatePrepareContext::new(source_file, file_id, body_range, string_table)?;
    let (content_header, config_owned_path_syntax_ids) = {
        let body_facts =
            TokenFactView::from_source_range(canonical, context.body_range).map_err(|error| {
                CompilerError::compiler_error(format!(
                    "Moth template body view is invalid: {error:?}"
                ))
            })?;
        let body_cursor = owner.cursor(context.body_range)?;
        let content_header = context.content_header(body_facts, string_table, path_fork)?;
        let config_owned_path_syntax_ids =
            if find_config_qualifier_marker_in_cursor(body_cursor, string_table, span_builder)
                .is_some()
            {
                (0..body_facts.len())
                    .filter_map(|index| {
                        body_facts.get(index).and_then(|token| token.path_syntax_id)
                    })
                    .collect()
            } else {
                Vec::new()
            };
        (content_header, config_owned_path_syntax_ids)
    };
    let mut headers = vec![content_header];
    let structural_file_references = classify_prepared_file_references(
        path_syntax.as_ref(),
        config_owned_path_syntax_ids,
        context.file_id,
        path_fork,
        string_table,
    );
    // Content sources can reference other content sources, so the synthetic constant's retained
    // body range takes the same token-level content ordering facts as authored shells.
    collect_content_source_ordering_hints(
        &mut headers,
        canonical,
        &structural_file_references,
        path_syntax.as_ref(),
        string_table,
        path_fork,
    )?;

    let source_token_stream = owner.into_tokens();
    Ok(FileFrontendPrepareOutput {
        source_file: context.source_file,
        file_id: context.file_id,
        path_syntax: PreparedFilePathSyntax::Preparing(path_syntax),
        token_count,
        token_stats,
        file_role: FileRole::Normal,
        file_dependency_clauses: Vec::new(),
        structural_file_references,
        dependency_selections: Vec::new(),
        headers,
        top_level_const_fragments: Vec::new(),
        source_token_stream: Some(source_token_stream),
        const_template_count: 0,
        runtime_fragment_count: 0,
        has_non_trivial_root_body: false,
        warnings: Vec::new(),
    })
}

/// File-local data needed to synthesize the normal constant header.
///
/// The body remains owned by the canonical source token stream. This context retains only its
/// semantic source path, checked range, and interned template directive.
struct MothTemplatePrepareContext {
    source_file: PathId,
    file_id: SourceId,
    body_range: TokenRange,
    markdown_directive: StringId,
}

impl MothTemplatePrepareContext {
    fn new(
        source_file: PathId,
        file_id: SourceId,
        body_range: TokenRange,
        string_table: &mut StringTable,
    ) -> Result<Self, CompilerError> {
        let markdown_directive = string_table.intern(MOTH_TEMPLATE_MARKDOWN_DIRECTIVE);

        Ok(Self {
            source_file,
            file_id,
            body_range,
            markdown_directive,
        })
    }

    fn body_range(canonical: &SourceTokens) -> Result<TokenRange, CompilerError> {
        let body_start = TokenIndex::try_from_index(1).ok_or_else(|| {
            CompilerError::compiler_error("Moth template body range start exceeded index space")
        })?;
        let body_end_index = canonical.len().checked_sub(1).ok_or_else(|| {
            CompilerError::compiler_error("Moth template token stream is missing its Eof token")
        })?;
        let body_end = TokenIndex::try_from_index(body_end_index).ok_or_else(|| {
            CompilerError::compiler_error("Moth template body range end exceeded index space")
        })?;
        canonical.range(body_start, body_end).map_err(|error| {
            CompilerError::compiler_error(format!("Moth template body range is invalid: {error:?}"))
        })
    }

    fn content_header(
        &self,
        body_tokens: TokenFactView<'_>,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<crate::compiler_frontend::headers::types::Header, CompilerError> {
        let initializer_references = collect_scanned_symbol_references(body_tokens, self.file_id);

        synthetic_content_header(
            SyntheticContentHeaderInput {
                source_file: self.source_file,
                file_id: self.file_id,
                initializer_range: self.body_range,
                payload: SyntheticContentPayload::MothTemplate {
                    markdown_directive: self.markdown_directive,
                },
                initializer_references,
            },
            string_table,
            path_fork,
        )
    }
}

#[cfg(test)]
#[path = "tests/moth_template_prepare_tests.rs"]
mod moth_template_prepare_tests;
