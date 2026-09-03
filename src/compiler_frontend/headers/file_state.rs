//! Per-file header parser state and output assembly.
//!
//! WHAT: owns the accumulators used while one token stream is split into declaration headers,
//! dependency records, const-fragment metadata, and implicit start-body tokens.
//! WHY: keeping mutable file-local state behind one owner lets `file_parser` read as the
//! high-level header-state machine instead of a long list of unrelated vectors and counters.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::types::{
    DependencySelection, FileFrontendPrepareError, FileFrontendPrepareOutput, FileRole, Header,
    HeaderExportMode, HeaderKind, PreparedFilePathSyntax, RetainedDependencyClause,
    TopLevelConstFragment,
};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
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
    pub(super) encountered_symbols: HashMap<StringId, SourceLocation>,
    pub(super) start_body_symbols: HashSet<StringId>,
    pub(super) start_function_body: Vec<Token>,
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
    /// The first `export:` block location, if this file has one.
    ///
    /// WHAT: enforces one public section per module-root file.
    /// WHY: public visibility is a file-level parser mode, not a lexical scope or a declaration
    /// prefix that can be reopened for a later group of items.
    pub(super) seen_export_block: Option<SourceLocation>,
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
            start_function_body: Vec::new(),
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

    pub(super) fn push_start_body_token(&mut self, token: Token) {
        self.start_function_body.push(token);
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

    pub(super) fn has_non_trivial_start_body(&self) -> bool {
        self.first_executable_start_body_location().is_some()
    }

    pub(super) fn first_executable_start_body_location(&self) -> Option<SourceLocation> {
        self.start_function_body
            .iter()
            .find(|token| {
                !matches!(
                    token.kind,
                    TokenKind::Eof | TokenKind::Newline | TokenKind::ModuleStart
                )
            })
            .map(|token| token.location.clone())
    }

    pub(super) fn into_non_entry_output(
        self,
        token_stream: &mut FileTokens,
        file_role: FileRole,
    ) -> Result<FileFrontendPrepareOutput, CompilerError> {
        let has_non_trivial_root_body =
            file_role == FileRole::ActiveModuleRoot && self.has_non_trivial_start_body();
        let path_syntax = PreparedFilePathSyntax::from_file_tokens(token_stream)?;
        Ok(FileFrontendPrepareOutput {
            source_file: token_stream.src_path.to_owned(),
            file_id: token_stream.file_id,
            path_syntax,
            token_count: self.token_count,
            token_stats: token_stream.token_stats,
            file_role,
            file_dependency_clauses: self.file_dependency_clauses,
            structural_file_references: Default::default(),
            dependency_selections: self.dependency_selections,
            canonical_os_path: token_stream.canonical_os_path.clone(),
            headers: self.headers,
            top_level_const_fragments: self.top_level_const_fragments,
            const_template_count: self.const_template_count,
            runtime_fragment_count: self.runtime_fragment_count,
            has_non_trivial_root_body,
            warnings: self.warnings,
        })
    }

    pub(super) fn into_entry_output(
        mut self,
        token_stream: &mut FileTokens,
        file_role: FileRole,
    ) -> Result<FileFrontendPrepareOutput, CompilerError> {
        let has_non_trivial_root_body = self.has_non_trivial_start_body();
        use crate::compiler_frontend::headers::types::HeaderExportMode;

        // Active module root: build the start function header for later AST body parsing.
        // `start` is never a dependency-graph participant, so this header keeps no graph edges.
        let start_tokens = FileTokens::new_substream(
            token_stream,
            token_stream.src_path.to_owned(),
            token_stream.file_id,
            self.start_function_body,
        );

        self.headers.push(Header {
            kind: HeaderKind::StartFunction,
            file_role,
            export_mode: HeaderExportMode::Private,
            local_ordering_hints: HashSet::new(),
            name_location: SourceLocation::new(
                token_stream.src_path.to_owned(),
                Default::default(),
                Default::default(),
            ),
            tokens: start_tokens,
            source_file: token_stream.src_path.to_owned(),
            capacity_references: Vec::new(),
        });

        let path_syntax = PreparedFilePathSyntax::from_file_tokens(token_stream)?;

        Ok(FileFrontendPrepareOutput {
            source_file: token_stream.src_path.to_owned(),
            file_id: token_stream.file_id,
            path_syntax,
            token_count: self.token_count,
            token_stats: token_stream.token_stats,
            file_role,
            file_dependency_clauses: self.file_dependency_clauses,
            structural_file_references: Default::default(),
            dependency_selections: self.dependency_selections,
            canonical_os_path: token_stream.canonical_os_path.clone(),
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
            diagnostic: Box::new(diagnostic),
        }
    }
}
