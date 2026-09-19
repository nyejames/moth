//! Per-file header parser state and output assembly.
//!
//! WHAT: owns the accumulators used while one token stream is split into declaration headers,
//! dependency records, const-fragment metadata, and source ranges for the implicit start body.
//! WHY: keeping mutable file-local state behind one owner lets `file_parser` read as the
//! high-level header-state machine instead of a long list of unrelated vectors and counters.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::types::{
    DependencySelection, FileFrontendPrepareError, FileFrontendPrepareOutput, FileRole, Header,
    HeaderExportMode, HeaderKind, PreparedFilePathSyntax, RetainedDependencyClause,
    SourceTokenOwner, TopLevelConstFragment,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
};
use std::sync::Arc;
use crate::projects::settings::{
    MINIMUM_LIKELY_DECLARATIONS, TOKEN_TO_DECLARATION_RATIO, TOKEN_TO_HEADER_RATIO,
};
use std::collections::{HashMap, HashSet};

/// Mutable parser state for one source file during header splitting.
///
/// WHAT: groups every accumulator whose lifetime is exactly one file parse.
/// WHY: these values are consumed together when building the per-file output, and keeping them
/// together makes branch handlers explicit about which state they mutate.
pub(super) struct HeaderFileParseState {
    pub(super) warnings: Vec<CompilerDiagnostic>,
    pub(super) headers: Vec<Header>,
    pub(super) encountered_symbols: HashMap<StringId, SourceSpan>,
    pub(super) start_body_symbols: HashSet<StringId>,
    /// Source-local token runs retained for the active root's implicit `start` body.
    pub(super) start_body_ranges: Vec<TokenRange>,
    has_non_trivial_start_body: bool,
    first_executable_start_body_span: Option<SourceSpan>,
    pub(super) file_dependency_clauses: Vec<RetainedDependencyClause>,
    pub(super) dependency_selections: Vec<DependencySelection>,
    /// Ordinal of the next authored dependency clause in this file.
    ///
    /// WHAT: one `DependencyShellId` per authored clause regardless of selected leaf count;
    ///       every authored clause keeps its own retained shell.
    pub(super) dependency_clause_count: usize,
    pub(super) top_level_const_fragments: Vec<TopLevelConstFragment>,
    pub(super) runtime_fragment_count: usize,
    pub(super) const_template_count: usize,
    /// The first `export:` block span, if this file has one.
    ///
    /// WHAT: enforces one public section per module-root file.
    /// WHY: public visibility is a file-level parser mode, not a lexical scope or a declaration
    /// prefix that can be reopened for a later group of items.
    pub(super) seen_export_block: Option<SourceSpan>,
    /// Current visibility mode while the top-level parser walks an `export:` block.
    pub(super) export_mode: HeaderExportMode,
    pub(super) export_block_item_count: usize,
    pub(super) token_count: usize,
}

impl HeaderFileParseState {
    pub(super) fn new(token_count: usize) -> Self {
        Self {
            warnings: Vec::new(),
            headers: Vec::with_capacity(token_count / TOKEN_TO_HEADER_RATIO),
            encountered_symbols: HashMap::with_capacity(
                MINIMUM_LIKELY_DECLARATIONS + (token_count / TOKEN_TO_DECLARATION_RATIO),
            ),
            start_body_symbols: HashSet::new(),
            start_body_ranges: Vec::new(),
            has_non_trivial_start_body: false,
            first_executable_start_body_span: None,
            file_dependency_clauses: Vec::new(),
            dependency_selections: Vec::new(),
            dependency_clause_count: 0,
            top_level_const_fragments: Vec::new(),
            runtime_fragment_count: 0,
            const_template_count: 0,
            seen_export_block: None,
            export_mode: HeaderExportMode::Private,
            export_block_item_count: 0,
            token_count,
        }
    }

    pub(super) fn record_start_body_token_ref(
        &mut self,
        token: TokenRef<'_>,
    ) -> Result<(), CompilerError> {
        let start = token.index();
        let end = TokenIndex::try_from_index(start.index().checked_add(1).ok_or_else(|| {
            CompilerError::compiler_error("start-body token index overflowed its source range")
        })?)
        .ok_or_else(|| {
            CompilerError::compiler_error("start-body token end exceeded its checked domain")
        })?;
        let range = TokenRange::new(token.source(), start, end)
            .ok_or_else(|| CompilerError::compiler_error("start-body token range was reversed"))?;
        self.record_start_body_range(range);
        self.observe_start_body_token_ref(token);
        Ok(())
    }

    pub(super) fn record_start_body_range(&mut self, range: TokenRange) {
        if let Some(previous) = self.start_body_ranges.last_mut()
            && previous.source() == range.source()
            && previous.end() == range.start()
        {
            *previous = TokenRange::new(previous.source(), previous.start(), range.end())
                .expect("merged ordered token ranges remain valid");
        } else {
            self.start_body_ranges.push(range);
        }
    }

    pub(super) fn record_start_body_source_range_from_source_tokens(
        &mut self,
        range: TokenRange,
        source: &SourceTokens,
    ) -> Result<(), CompilerError> {
        let cursor = source.cursor(range).map_err(|_| {
            CompilerError::compiler_error("start-body range exceeds its source token owner")
        })?;
        self.record_start_body_range(range);
        self.observe_start_body_cursor(cursor);
        Ok(())
    }

    pub(super) fn observe_start_body_cursor(&mut self, mut cursor: TokenCursor<'_>) {
        while let Some(token) = cursor.advance() {
            self.observe_start_body_token_ref(token);
            if token.is_eof() {
                break;
            }
        }
    }

    pub(super) fn observe_start_body_token_ref(&mut self, token: TokenRef<'_>) {
        let tag = token.tag();
        if tag != TokenTag::EOF && tag != TokenTag::NEWLINE && tag != TokenTag::MODULE_START {
            self.has_non_trivial_start_body = true;
            if self.first_executable_start_body_span.is_none() {
                self.first_executable_start_body_span = Some(token.source_span());
            }
        }
    }

    pub(super) fn has_non_trivial_start_body(&self) -> bool {
        self.has_non_trivial_start_body
    }

    pub(super) fn first_executable_start_body_span(&self) -> Option<SourceSpan> {
        self.first_executable_start_body_span
    }

    pub(super) fn register_start_body_symbol(&mut self, name_id: StringId) {
        self.start_body_symbols.insert(name_id);
    }

    pub(super) fn register_header(&mut self, header: Header) {
        self.headers.push(header);
    }

    pub(super) fn register_top_level_const_fragment(
        &mut self,
        fragment: TopLevelConstFragment,
        header: Header,
    ) {
        self.top_level_const_fragments.push(fragment);
        self.headers.push(header);
    }

    pub(super) fn into_non_entry_output(
        self,
        owner: SourceTokenOwner,
        path_syntax: Arc<PathSyntaxTable>,
        file_role: FileRole,
    ) -> Result<FileFrontendPrepareOutput, CompilerError> {
        let file_id = owner.source_id();
        let source_file = owner.logical_path();
        let canonical_os_path = owner.os_path_cloned();
        let has_non_trivial_root_body =
            file_role == FileRole::ActiveModuleRoot && self.has_non_trivial_start_body();
        let token_stats = owner.token_stats();
        let source_token_stream = owner.into_tokens();
        Ok(FileFrontendPrepareOutput {
            source_file,
            file_id,
            path_syntax: PreparedFilePathSyntax::Preparing(path_syntax),
            token_count: self.token_count,
            token_stats,
            file_role,
            file_dependency_clauses: self.file_dependency_clauses,
            structural_file_references: Default::default(),
            dependency_selections: self.dependency_selections,
            canonical_os_path,
            headers: self.headers,
            top_level_const_fragments: self.top_level_const_fragments,
            source_token_stream: Some(source_token_stream),
            const_template_count: self.const_template_count,
            runtime_fragment_count: self.runtime_fragment_count,
            has_non_trivial_root_body,
            warnings: self.warnings,
        })
    }

    pub(super) fn into_entry_output(
        mut self,
        mut owner: SourceTokenOwner,
        path_syntax: Arc<PathSyntaxTable>,
        end_index: TokenIndex,
        file_role: FileRole,
    ) -> Result<FileFrontendPrepareOutput, CompilerError> {
        let file_id = owner.source_id();
        let source_file = owner.logical_path();
        let canonical_os_path = owner.os_path_cloned();
        let has_non_trivial_root_body = self.has_non_trivial_start_body();
        use crate::compiler_frontend::headers::types::HeaderExportMode;

        // Active module root: publish the source-owned start sequence for later AST body parsing.
        // `start` is never a dependency-graph participant, so this header keeps no graph edges.
        let token_sequence = owner.register_token_sequence(&self.start_body_ranges)?;
        let start_range = TokenRange::new(file_id, end_index, end_index)
            .expect("equal token indexes always form a valid empty range");

        self.headers.push(Header {
            kind: HeaderKind::StartFunction,
            file_role,
            export_mode: HeaderExportMode::Private,
            local_ordering_hints: HashSet::new(),
            name_span: None,
            synthetic_content_payload: None,
            tokens: start_range,
            declaration_path: source_file,
            token_sequence: Some(token_sequence),
            capacity_references: Vec::new(),
        });

        let token_stats = owner.token_stats();
        let source_token_stream = owner.into_tokens();
        Ok(FileFrontendPrepareOutput {
            source_file,
            file_id,
            path_syntax: PreparedFilePathSyntax::Preparing(path_syntax),
            token_count: self.token_count,
            source_token_stream: Some(source_token_stream),
            token_stats,
            file_role,
            file_dependency_clauses: self.file_dependency_clauses,
            structural_file_references: Default::default(),
            dependency_selections: self.dependency_selections,
            canonical_os_path,
            headers: self.headers,
            top_level_const_fragments: self.top_level_const_fragments,
            const_template_count: self.const_template_count,
            runtime_fragment_count: self.runtime_fragment_count,
            has_non_trivial_root_body,
            warnings: self.warnings,
        })
    }

    pub(super) fn into_error(self, diagnostic: CompilerDiagnostic) -> FileFrontendPrepareError {
        FileFrontendPrepareError {
            warnings: self.warnings,
            diagnostic,
        }
    }
}
