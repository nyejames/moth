//! Synthetic Moth template header preparation.
//!
//! WHAT: turns a tokenized `.mtf` body into the normal private `content #String`
//! declaration consumed by dependency sorting and AST.
//! WHY: Moth template source is authored as a template body, but later frontend stages
//! should see an ordinary constant header instead of a Moth template-specific AST path
//! or textually wrapped source.
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::qualified_references::{
    HeaderExpressionRange, collect_qualified_references,
};
use crate::compiler_frontend::headers::synthetic_content_header::{
    SyntheticContentHeaderInput, synthetic_content_header,
};
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, FileRole, PreparedFilePathSyntax,
    SourceTokenOwner, SyntheticContentPayload,
};
use crate::compiler_frontend::paths::file_references::classify_prepared_file_references;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;

use crate::compiler_frontend::source::SourceId;
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
///
/// WHAT: returns the ordinary per-file preparation failure lane, not a bare infrastructure result.
/// WHY: template-head qualified references validate their authored path and alias shape exactly
///      like ordinary `.moth` expression positions, so their typed diagnostics must reach the same
///      user-facing boundary instead of being downgraded to infrastructure failures.
pub(crate) fn prepare_moth_template_file(
    owner: SourceTokenOwner,
    source_file: PathId,
    path_syntax: Arc<PathSyntaxTable>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let file_id = owner.source_id();
    #[cfg(feature = "data_layout_memory_probe")]
    crate::compiler_frontend::instrumentation::record_source_tokens_path_table(
        owner.tokens_ref(),
        &path_syntax,
    );
    let canonical = owner.tokens_ref();
    if canonical.source() != file_id {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "Moth template source token owner does not match its file identity",
        )
        .into());
    }
    let token_count = owner.len();
    let token_stats = owner.token_stats();
    let body_range = MothTemplatePrepareContext::body_range(canonical)?;
    let context = MothTemplatePrepareContext::new(source_file, file_id, body_range, string_table);
    let (content_header, consumed_path_syntax_ids, qualified_references) = {
        let body_facts =
            TokenFactView::from_source_range(canonical, context.body_range).map_err(|error| {
                InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    format!("Moth template body view is invalid: {error:?}"),
                )
            })?;
        let content_header = context.content_header(body_facts, string_table, path_fork)?;
        // Every `.mtf` qualified occurrence lives inside a template head (including nested
        // heads), so one bounded scan of the retained body is the complete occurrence set.
        // The synthetic content body is compile-time content, so every occurrence is
        // const-required.
        let occurrences = collect_qualified_references(
            canonical,
            [HeaderExpressionRange::const_required(0, context.body_range)],
            context.file_id,
            0,
            path_syntax.as_ref(),
            string_table,
            path_fork,
        )
        .map_err(|failure| {
            FileFrontendPrepareFailure::from_header_parse(failure, DiagnosticBag::new())
        })?;
        let consumed_path_syntax_ids: Vec<_> = occurrences
            .iter()
            .map(|occurrence| occurrence.dependency.path_syntax)
            .collect();
        (content_header, consumed_path_syntax_ids, occurrences)
    };
    let mut headers = vec![content_header];
    let structural_file_references = classify_prepared_file_references(
        path_syntax.as_ref(),
        consumed_path_syntax_ids,
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
        qualified_references,
        dependency_selections: Vec::new(),
        headers,
        top_level_const_fragments: Vec::new(),
        source_token_stream: Some(source_token_stream),
        const_template_count: 0,
        runtime_fragment_count: 0,
        start_body_binding_spans: rustc_hash::FxHashMap::default(),
        warnings: DiagnosticBag::new(),
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
    ) -> Self {
        let markdown_directive = string_table.intern(MOTH_TEMPLATE_MARKDOWN_DIRECTIVE);

        Self {
            source_file,
            file_id,
            body_range,
            markdown_directive,
        }
    }

    fn body_range(canonical: &SourceTokens) -> Result<TokenRange, InfrastructureFailure> {
        let body_start = TokenIndex::try_from_index(1).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "Moth template body range start exceeded index space",
            )
        })?;
        let body_end_index = canonical.len().checked_sub(1).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "Moth template token stream is missing its Eof token",
            )
        })?;
        let body_end = TokenIndex::try_from_index(body_end_index).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "Moth template body range end exceeded index space",
            )
        })?;
        canonical.range(body_start, body_end).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("Moth template body range is invalid: {error:?}"),
            )
        })
    }

    fn content_header(
        &self,
        body_tokens: TokenFactView<'_>,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<crate::compiler_frontend::headers::types::Header, InfrastructureFailure> {
        let initializer_references =
            collect_scanned_symbol_references(body_tokens, self.file_id, string_table);

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
