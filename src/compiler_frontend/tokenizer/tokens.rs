//! Token definitions and source-location primitives for the frontend tokenizer.
//!
//! WHAT: defines token kinds, token records, and the location metadata threaded through parsing.
//! WHY: every frontend stage past lexing depends on one canonical token and location model.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::numeric_text::store::{NumericLiteralId, NumericLiteralStore};
use crate::compiler_frontend::numeric_text::token::{
    NumericLiteralKind, NumericLiteralToken,
};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

use crate::token_log;
use std::iter::Peekable;
use std::ops::Deref;
use std::path::PathBuf;
use std::str::Chars;
use std::sync::Arc;

/// Entry policy for one tokenizer invocation.
///
/// `TokenizeMode` remains the current lexical state while tokenization is
/// running. This type only decides which lexical state the stream starts in and
/// how the initial frame should behave when the source is a synthetic template
/// body such as Moth template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenizerEntryMode {
    SourceFile,
    TemplateBody {
        initial_close_policy: InitialTemplateClosePolicy,
    },
}

impl TokenizerEntryMode {
    fn initial_tokenize_mode(self) -> TokenizeMode {
        match self {
            Self::SourceFile => TokenizeMode::Normal,
            Self::TemplateBody { .. } => TokenizeMode::TemplateBody,
        }
    }

    /// Returns the tokenizer entry mode for a source kind, if any.
    ///
    /// WHAT: maps source kinds that need tokenization to their entry policy.
    /// WHY: some compiler-recognized source kinds such as plain Markdown are content assets and
    ///      must not be tokenized as Moth syntax.
    ///
    /// `None` means the source kind is compiler-recognized but has no tokenizer path.
    pub fn for_source_file_kind(source_kind: SourceFileKind) -> Option<Self> {
        match source_kind {
            SourceFileKind::Moth => Some(Self::SourceFile),
            SourceFileKind::MothTemplate => Some(Self::TemplateBody {
                initial_close_policy: InitialTemplateClosePolicy::RejectOuterClose { source_kind },
            }),
            SourceFileKind::PlainMarkdown => None,
        }
    }
}

/// Close-delimiter policy for tokenizers that start inside a template body.
///
/// Normal authored templates can close their own opening `[`. Synthetic entry
/// bodies have no authored outer `[`, so the initial `]` should be rejected
/// instead of letting the stream silently escape to source-file mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialTemplateClosePolicy {
    Allow,
    RejectOuterClose { source_kind: SourceFileKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenizeMode {
    Normal,
    TemplateBody,
    TemplateHead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TemplateBodyMode {
    #[default]
    Normal,
    Balanced,
    DiscardBalanced,
}

impl TemplateBodyMode {
    pub fn is_balanced_mode(self) -> bool {
        matches!(
            self,
            TemplateBodyMode::Balanced | TemplateBodyMode::DiscardBalanced
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: LocalSpan,
}

impl Token {
    /// Construct a token from its source-local exact span.
    pub fn new(kind: TokenKind, span: LocalSpan) -> Self {
        Self { kind, span }
    }
}

/// The path-table lifecycle for one token stream.
///
/// WHAT: a tokenized source keeps the only mutable table owner while header syntax is being
///       prepared. Header substreams defer their table reference until the prepared-file owner
///       has completed its one string remap and source-identity rebind, then share the frozen
///       immutable table.
/// WHY: an `Arc` is used only after the construction owner has finished mutating the table. This
///      prevents ordinary substreams from copying rows while also preventing copy-on-write or
///      mutable shared path tables during final preparation.
#[derive(Clone, Debug)]
pub enum FilePathSyntax {
    Preparing(Arc<PathSyntaxTable>),
    Deferred,
    Shared(Arc<PathSyntaxTable>),
}

impl FilePathSyntax {
    fn preparing(table: PathSyntaxTable) -> Self {
        Self::Preparing(Arc::new(table))
    }

    fn shared(mut table: PathSyntaxTable) -> Self {
        table.freeze();
        Self::Shared(Arc::new(table))
    }

    fn permanent_substream(&self) -> Self {
        match self {
            // Header syntax does not inspect retained body tokens while parsing the file. Keep
            // the body deferred so the prepared-file owner remains the sole mutable table owner.
            Self::Preparing(_) | Self::Deferred => Self::Deferred,
            Self::Shared(table) => Self::Shared(Arc::clone(table)),
        }
    }

    fn frozen_substream(&self) -> Result<Self, CompilerError> {
        match self {
            Self::Shared(table) => Ok(Self::Shared(Arc::clone(table))),
            Self::Preparing(_) => Err(CompilerError::compiler_error(
                "retained AST substream requested a path table before the prepared file froze",
            )),
            Self::Deferred => Err(CompilerError::compiler_error(
                "retained AST substream requested a path table before it was attached",
            )),
        }
    }

    fn table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        match self {
            Self::Preparing(table) | Self::Shared(table) => Ok(table),
            Self::Deferred => Err(CompilerError::compiler_error(
                "retained token stream was read before its prepared-file path table froze",
            )),
        }
    }

    fn preparing_table_mut(&mut self) -> Result<&mut PathSyntaxTable, CompilerError> {
        let Self::Preparing(table) = self else {
            return Err(CompilerError::compiler_error(
                "path-table mutation was requested after the file preparation owner froze",
            ));
        };

        Arc::get_mut(table).ok_or_else(|| {
            CompilerError::compiler_error(
                "path-table mutation was requested while a temporary parser still held a shared view",
            )
        })
    }

    fn take_preparing_table(&mut self) -> Result<Arc<PathSyntaxTable>, CompilerError> {
        let state = std::mem::replace(self, Self::Deferred);
        match state {
            Self::Preparing(table) => Ok(table),
            Self::Deferred => Err(CompilerError::compiler_error(
                "prepared-file output attempted to take a path table from a deferred stream",
            )),
            Self::Shared(_) => Err(CompilerError::compiler_error(
                "prepared-file output attempted to take an already-frozen path table",
            )),
        }
    }

    fn require_deferred(&self) -> Result<(), CompilerError> {
        if matches!(self, Self::Deferred) {
            return Ok(());
        }

        Err(CompilerError::compiler_error(
            "prepared-file header stream must remain deferred until its file-owned path table freezes",
        ))
    }

    fn require_shared_table(&self, expected: &Arc<PathSyntaxTable>) -> Result<(), CompilerError> {
        match self {
            Self::Shared(table) if Arc::ptr_eq(table, expected) => Ok(()),
            Self::Shared(_) => Err(CompilerError::compiler_error(
                "prepared-file header stream shares a different path table than its file owner",
            )),
            Self::Preparing(_) => Err(CompilerError::compiler_error(
                "prepared-file header stream retained a mutable path table after file freeze",
            )),
            Self::Deferred => Err(CompilerError::compiler_error(
                "prepared-file header stream was not attached to the frozen file-owned path table",
            )),
        }
    }

    /// Attach after a whole-file preflight has proven this stream is deferred.
    ///
    /// This intentionally has no fallible branch. `FileFrontendPrepareOutput` first checks every
    /// retained header, then changes the file owner to frozen and attaches all streams in one
    /// non-failing commit section. Adding an `Arc` clone never copies table rows or enables COW.
    fn attach_preflighted_shared(&mut self, table: Arc<PathSyntaxTable>) {
        debug_assert!(matches!(self, Self::Deferred));
        *self = Self::Shared(table);
    }
}

impl Deref for FilePathSyntax {
    type Target = PathSyntaxTable;

    fn deref(&self) -> &Self::Target {
        // Every production consumer runs after the prepared-file freeze boundary. Reaching this
        // point while deferred is therefore an internal lifecycle violation, while public
        // validation APIs use `FileTokens::path_syntax_table` and return `CompilerError` instead.
        self.table()
            .expect("path syntax must be attached before a retained token stream is consumed")
    }
}

#[derive(Clone, Debug)]
pub struct FileTokens {
    pub tokens: Vec<Token>,
    /// File-owned authored path trees referenced by `TokenKind::Path` handles.
    pub path_syntax: FilePathSyntax,
    /// Source-owned numeric lexical records. `TokenKind::NumericLiteral` remains a transient
    /// compatibility adapter until parser consumers migrate to these handles.
    pub(crate) numeric_literals: NumericLiteralStore,
    /// Numeric side-store handle for each token position, when that token is numeric.
    pub(crate) numeric_literal_ids: Vec<Option<NumericLiteralId>>,
    /// Complete logical identity of the owning source file in the active path table.
    pub src_path: PathId,
    /// Required owning source identity for every token stream, including materialised generics.
    pub file_id: SourceId,
    /// Canonical filesystem source path for IO/path-resolution-only logic.
    pub canonical_os_path: Option<PathBuf>,
    pub(crate) token_stats: TokenStats,
    pub index: usize,
    pub length: usize,
}

impl FileTokens {
    #[cfg(test)]
    pub fn new(src_path: PathId, file_id: SourceId, tokens: Vec<Token>) -> FileTokens {
        Self::new_with_identity(src_path, file_id, None, tokens, PathSyntaxTable::new())
    }

    /// Construct the sole mutable path/numeric-store owner for a newly tokenized source file.
    pub fn new_with_identity(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
    ) -> FileTokens {
        Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::preparing(path_syntax),
        )
    }

    /// Source-boundary constructor used by the lexer after numeric staging has completed.
    pub(crate) fn new_with_identity_and_numeric_store(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
        numeric_literals: NumericLiteralStore,
    ) -> FileTokens {
        let numeric_literal_ids = numeric_ids_for_staged_store(&tokens, &numeric_literals);
        Self::with_path_syntax_and_numeric_store(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::preparing(path_syntax),
            numeric_literals,
            numeric_literal_ids,
        )
    }

    /// Construct a stream from an already-frozen table owned by a generated persistent artefact.
    pub(crate) fn new_frozen(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
    ) -> FileTokens {
        let mut stream = Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::shared(path_syntax),
        );
        stream.freeze_numeric_literals();
        stream
    }

    pub(crate) fn new_frozen_with_numeric_store(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
        mut numeric_literals: NumericLiteralStore,
        numeric_literal_ids: Vec<Option<NumericLiteralId>>,
    ) -> FileTokens {
        numeric_literals.freeze();
        Self::with_path_syntax_and_numeric_store(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::shared(path_syntax),
            numeric_literals,
            numeric_literal_ids,
        )
    }

    /// Construct a retained token stream that will receive its table from the completed
    /// prepared-file owner.
    pub fn new_deferred_with_identity(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
    ) -> FileTokens {
        Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::Deferred,
        )
    }

    fn with_path_syntax(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
    ) -> FileTokens {
        let (numeric_literals, numeric_literal_ids) =
            numeric_store_from_tokens(file_id, &tokens);
        Self::with_path_syntax_and_numeric_store(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            path_syntax,
            numeric_literals,
            numeric_literal_ids,
        )
    }

    fn with_path_syntax_and_numeric_store(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
        numeric_literals: NumericLiteralStore,
        numeric_literal_ids: Vec<Option<NumericLiteralId>>,
    ) -> FileTokens {
        debug_assert_eq!(
            tokens.len(),
            numeric_literal_ids.len(),
            "numeric side-store handles must align with token positions"
        );
        FileTokens {
            length: tokens.len(),
            path_syntax,
            numeric_literals,
            numeric_literal_ids,
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            token_stats: TokenStats::default(),
            index: 0,
        }
    }
    /// Build a permanent sub-stream over a token slice.
    ///
    /// Header bodies defer the path-table attachment until the prepared-file owner freezes.
    /// Later AST substreams clone only the immutable table handle.
    pub fn new_substream(
        source: &FileTokens,
        src_path: PathId,
        file_id: SourceId,
        tokens: Vec<Token>,
    ) -> FileTokens {
        Self::with_path_syntax(
            src_path,
            file_id,
            source.canonical_os_path.clone(),
            tokens,
            source.path_syntax.permanent_substream(),
        )
    }

    /// Build a short-lived parser stream for syntax that cannot contain path handles.
    ///
    /// WHAT: provides speculative parsers with token and source-location identity but no
    ///       `PathSyntaxTable` access.
    /// WHY: type-slice parsing reuses the ordinary type grammar while splitting collection
    ///      syntax. That grammar never reads `TokenKind::Path`, so acquiring the prepared file's
    ///      mutable table would add a fallible lifecycle edge and temporarily prevent the real
    ///      file owner from remapping or rebinding its one table.
    pub(crate) fn new_path_free_substream(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
    ) -> FileTokens {
        Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::Deferred,
        )
    }

    /// Build a downstream parser stream over already-frozen file syntax.
    ///
    /// AST consumers use this for defaults, declaration initializers and loop headers. The table
    /// handle is cloned, while path rows and their dense IDs remain owned by the prepared source.
    /// The caller supplies the owning `SourceId` (for generated bodies, the retained donor/owner
    /// identity); no `None` or magic identity is accepted. The rebuilt source-local numeric
    /// store is frozen eagerly because the source table is already immutable: there is no
    /// later owner-boundary remap for these transient parser streams.
    pub fn new_from_slice(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        source_path_syntax: &FilePathSyntax,
    ) -> Result<FileTokens, CompilerError> {
        let mut stream = Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            source_path_syntax.frozen_substream()?,
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }
    /// Return the canonical path table once the stream has reached a readable lifecycle state.
    pub fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        self.path_syntax.table()
    }
    /// Return the source-owned numeric cold store after construction.
    pub(crate) fn numeric_literal_store(&self) -> &NumericLiteralStore {
        &self.numeric_literals
    }

    pub(crate) fn numeric_literal_ids(&self) -> &[Option<NumericLiteralId>] {
        &self.numeric_literal_ids
    }

    pub(crate) fn numeric_literal_id_at(&self, token_index: usize) -> Option<NumericLiteralId> {
        self.numeric_literal_ids.get(token_index).copied().flatten()
    }

    /// Move the sole mutable path-table owner into a prepared-file output.
    pub(crate) fn take_preparing_path_syntax(
        &mut self,
    ) -> Result<Arc<PathSyntaxTable>, CompilerError> {
        self.path_syntax.take_preparing_table()
    }

    /// Verify that a retained header has not received a file-owned table before the output's
    /// atomic freeze boundary.
    pub(crate) fn require_deferred_path_syntax(&self) -> Result<(), CompilerError> {
        self.path_syntax.require_deferred()
    }

    /// Verify that a frozen retained header points at this exact immutable table allocation.
    pub(crate) fn require_shared_path_syntax(
        &self,
        path_syntax: &Arc<PathSyntaxTable>,
    ) -> Result<(), CompilerError> {
        self.path_syntax.require_shared_table(path_syntax)
    }

    /// Commit a table attachment after `require_deferred_path_syntax` passed for every header.
    pub(crate) fn attach_preflighted_shared_path_syntax(
        &mut self,
        path_syntax: Arc<PathSyntaxTable>,
    ) {
        self.path_syntax.attach_preflighted_shared(path_syntax);
    }

    /// Freeze the ordinary source-owned numeric store at publication.
    ///
    /// The caller must complete all construction-time string remaps first. Generic
    /// materialisation's remapped clone finishes frozen through its own checked owner
    /// boundary and never calls this on an already-frozen store.
    pub(crate) fn freeze_numeric_literals(&mut self) {
        self.numeric_literals.freeze();
    }

    /// Freeze a standalone token stream used by an AST-focused unit test.
    ///
    /// Production preparation moves the mutable table into `FileFrontendPrepareOutput`, validates
    /// the complete file, and attaches the resulting immutable table to retained headers. These
    /// direct parser tests have no retained header output, but they still model the AST's
    /// post-freeze input contract rather than letting parser substreams read a mutable table.
    #[cfg(test)]
    pub(crate) fn freeze_path_syntax_for_test(&mut self) {
        let path_syntax = std::mem::replace(&mut self.path_syntax, FilePathSyntax::Deferred);
        self.path_syntax = match path_syntax {
            FilePathSyntax::Preparing(mut path_syntax)
            | FilePathSyntax::Shared(mut path_syntax) => {
                Arc::get_mut(&mut path_syntax)
                    .expect("test path table unexpectedly had another owner")
                    .freeze();
                FilePathSyntax::Shared(path_syntax)
            }
            FilePathSyntax::Deferred => {
                panic!("test token stream did not retain a file-owned path table to freeze")
            }
        };
        self.freeze_numeric_literals();
    }

    pub fn current_token_kind(&self) -> &TokenKind {
        &self.tokens[self.index].kind
    }

    pub fn current_token(&self) -> Token {
        self.tokens[self.index].clone()
    }

    /// This should never be called from a context where there is no previous token
    pub fn previous_token(&self) -> &TokenKind {
        &self.tokens[self.index - 1].kind
    }

    pub fn peek_next_token(&self) -> Option<&TokenKind> {
        if self.index + 1 >= self.length {
            return None;
        }
        self.tokens.get(self.index + 1).map(|token| &token.kind)
    }
    /// Return the exact global span of the current token.
    pub fn current_span(&self) -> SourceSpan {
        SourceSpan::new(self.file_id, self.tokens[self.index].span)
    }

    /// Return the exact global span of the current postfix operator token.
    ///
    /// Postfix operators are already represented by one-byte token spans. Unlike the removed
    /// character-position location bridge, this operation does not rewind or infer columns.
    pub fn current_postfix_operator_span(&self) -> SourceSpan {
        self.current_span()
    }

    pub fn advance(&mut self) {
        if self.index >= self.tokens.len() {
            token_log!(Red "Compiler tried to advance past token stream bounds");
            return;
        }

        match &self.current_token_kind() {
            // Can't advance past End of File
            &TokenKind::Eof => {
                // Show a warning for compiler_frontend development purposes
                token_log!(Red "Compiler tried to advance past EOF");
            }

            _ => {
                self.index += 1;
            }
        }
    }

    pub fn skip_newlines(&mut self) {
        while self.index + 1 < self.length
            && matches!(self.current_token_kind(), TokenKind::Newline)
        {
            self.index += 1;
        }
    }

    /// WHAT: updates every token's kind after a string-table merge.
    /// WHY: tokenization produces per-file local string IDs that must be rewritten before
    ///      module-wide stages consume the token stream.
    ///
    /// NOTE: `canonical_os_path` is intentionally NOT remapped; it is a filesystem identity
    ///       (`PathBuf`), not an interned string identity. `src_path` is a `PathId` owned
    ///       by the active path fork and remaps through `remap_path_ids`, never through the
    ///       string table.
    // This is wired when file-level frontend outputs are merged before module-wide header
    // aggregation. Keeping it beside token remapping makes the traversal owner explicit.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if self.numeric_literals.is_frozen() {
            panic!("numeric literal remapping was requested after the source publication freeze");
        }
        self.numeric_literals.remap_string_ids(remap);
        self.remap_token_payload_string_ids(remap);
        // Path tokens carry dense handles. The prepared-file output owns the single path-table
        // remap, so substreams remap only their local token and semantic-path payloads here.
    }

    /// Remap this stream's complete-path identity after its path fork merges.
    ///
    /// WHAT: rewrites `src_path` through the worker-local merge remap.
    /// WHY: the file identity is interned against a chunk-local fork; the canonical chunk
    ///      merge re-interns those nodes into the module fork. Token payload strings remap
    ///      separately through `remap_string_ids`; path-table rows remap through the
    ///      prepared-file output that owns the sole mutable table.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.src_path = remap.get(self.src_path);
    }

    fn remap_token_payload_string_ids(&mut self, remap: &StringIdRemap) {
        for token in &mut self.tokens {
            token.remap_string_ids(remap);
        }
    }

    #[cfg(test)]
    /// Remap a token stream while it still owns its mutable path table.
    pub(crate) fn remap_preparing_string_ids(
        &mut self,
        remap: &StringIdRemap,
    ) -> Result<(), CompilerError> {
        // Validate the mutable lifecycle before changing any token payload. The second access is
        // safe because the first borrow ends before the payload traversal begins.
        self.path_syntax.preparing_table_mut()?;
        if self.numeric_literals.is_frozen() {
            return Err(CompilerError::compiler_error(
                "numeric literal remapping was requested after the source publication freeze",
            ));
        }
        self.numeric_literals.remap_string_ids(remap);
        self.remap_token_payload_string_ids(remap);
        Ok(())
    }

    #[cfg(test)]
    /// Remap a preparing stream's file identity and its mutable path table.
    pub(crate) fn remap_preparing_path_ids(
        &mut self,
        remap: &PathIdRemap,
    ) -> Result<(), CompilerError> {
        self.path_syntax.preparing_table_mut()?;
        self.src_path = remap.get(self.src_path);
        self.path_syntax
            .preparing_table_mut()?
            .remap_path_ids(remap);
        Ok(())
    }

    /// Rebind this token stream to a new module source identity.
    ///
    /// Source-local token spans remain unchanged. Only the owning `SourceId` and path-table rows
    /// are restamped, so every global path span continues to name the same byte range.
    pub fn rebind_source_identity(
        &mut self,
        logical_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
    ) -> Result<(), CompilerError> {
        self.path_syntax.preparing_table_mut()?;
        self.src_path = logical_path;
        self.rebind_file_identity(self.src_path, file_id, canonical_os_path);
        self.path_syntax
            .preparing_table_mut()?
            .rebind_source_identity(file_id);
        Ok(())
    }

    /// Rebind file-owned identity while preserving this stream's semantic path.
    ///
    /// Token spans are source-local and therefore remain unchanged. The owner identity is stored
    /// once on `FileTokens`; path rows are restamped by `rebind_source_identity` before publication.
    /// The numeric cold store keeps source-local rows and only its owner is restamped, mirroring
    /// the path table.
    pub fn rebind_file_identity(
        &mut self,
        _logical_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
    ) {
        self.file_id = file_id;
        self.canonical_os_path = canonical_os_path;
        self.numeric_literals.rebind_source_identity(file_id);
    }
}

fn numeric_store_from_tokens(
    source: SourceId,
    tokens: &[Token],
) -> (NumericLiteralStore, Vec<Option<NumericLiteralId>>) {
    let mut store = NumericLiteralStore::with_source(source);
    let mut ids = Vec::with_capacity(tokens.len());
    for token in tokens {
        let id = match &token.kind {
            TokenKind::NumericLiteral(literal) => Some(
                store
                    .try_push_for_source(source, literal.clone())
                    .expect("test/retained numeric literal row must fit its checked handle domain"),
            ),
            _ => None,
        };
        ids.push(id);
    }
    (store, ids)
}

/// Align one handle per numeric position from a lexer-staged numeric store.
///
/// The staged store was pushed once per numeric token in lexer order, so handles are assigned
/// positionally without re-reading token payloads. Any length mismatch is a lifecycle violation.
fn numeric_ids_for_staged_store(
    tokens: &[Token],
    store: &NumericLiteralStore,
) -> Vec<Option<NumericLiteralId>> {
    let mut ids = Vec::with_capacity(tokens.len());
    let mut next = 0usize;
    for token in tokens {
        if !matches!(token.kind, TokenKind::NumericLiteral(_)) {
            ids.push(None);
            continue;
        }
        let id = NumericLiteralId::try_from_index(next)
            .expect("lexer-staged numeric literal row must fit its checked handle domain");
        debug_assert!(
            id.index().is_some_and(|index| index < store.len()),
            "lexer-staged numeric handle must address a staged row"
        );
        ids.push(Some(id));
        next = next.saturating_add(1);
    }
    ids
}

pub struct TokenStream<'a> {
    pub file_id: SourceId,
    pub chars: Peekable<Chars<'a>>,
    /// Byte offset of the next character to consume.
    pub byte_offset: u32,
    pub start_byte_offset: u32,
    /// Byte offset of the character most recently consumed.
    ///
    /// The lexer reads a token's first character before deciding the token starts here, so the
    /// start of that character is the only correct byte start.
    pub last_char_start: u32,
    pub mode: TokenizeMode,
    // nested template heads can appear while parsing another template head/body,
    // and parent/child templates can have different style directives. We therefore
    // keep code-specific state on the current template frame and pop it naturally
    // when that template closes.
    pub template_mode_stack: Vec<TemplateModeFrame>,
    /// Path syntax rows built while lexing; moved into `FileTokens` when tokenization
    /// completes.
    pub path_syntax: PathSyntaxTable,
    /// Numeric literal records staged during lexing and moved into `FileTokens` at completion.
    pub numeric_literals: NumericLiteralStore,
    /// One mutable extended-span builder borrowed from the caller for every token encoded by
    /// this source stream.
    ///
    /// The caller keeps ownership across tokenization, so rows appended by one pass remain
    /// visible to the next and survive both success and diagnostic exits. The builder stays
    /// outside [`FileTokens`] because parser substreams clone that value.
    pub extended_span_builder: &'a mut ExtendedSpanBuilder,
}

// WHAT: Metadata for one template nesting level in the tokenizer.
//
// WHY: directives are declared in a template head, but affect only that template's
// body tokenization. This frame carries that intent across `:` (head -> body),
// tracks bracket balance for balanced body modes, carries initial-frame close
// policy, and ensures nested templates cannot accidentally inherit or overwrite
// the parent's body behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemplateModeFrame {
    pub mode: TokenizeMode,
    pub body_mode: TemplateBodyMode,
    pub body_open_square_brackets: usize,
    pub body_closed_square_brackets: usize,
    pub initial_close_policy: InitialTemplateClosePolicy,
}

impl TemplateModeFrame {
    fn new(mode: TokenizeMode) -> Self {
        Self {
            mode,
            body_mode: TemplateBodyMode::Normal,
            body_open_square_brackets: 0,
            body_closed_square_brackets: 0,
            initial_close_policy: InitialTemplateClosePolicy::Allow,
        }
    }

    fn initial(mode: TokenizeMode, close_policy: InitialTemplateClosePolicy) -> Self {
        Self {
            initial_close_policy: close_policy,
            ..Self::new(mode)
        }
    }
}

impl<'a> TokenStream<'a> {
    pub fn new(
        source_code: &'a str,
        file_id: SourceId,
        entry_mode: TokenizerEntryMode,
        extended_span_builder: &'a mut ExtendedSpanBuilder,
    ) -> Self {
        let mode = entry_mode.initial_tokenize_mode();
        let initial_close_policy = match entry_mode {
            TokenizerEntryMode::SourceFile => InitialTemplateClosePolicy::Allow,
            TokenizerEntryMode::TemplateBody {
                initial_close_policy,
            } => initial_close_policy,
        };

        Self {
            file_id,
            chars: source_code.chars().peekable(),
            byte_offset: 0,
            start_byte_offset: 0,
            last_char_start: 0,
            mode,
            template_mode_stack: vec![TemplateModeFrame::initial(mode, initial_close_policy)],
            path_syntax: PathSyntaxTable::with_source(file_id),
            numeric_literals: NumericLiteralStore::with_source(file_id),
            extended_span_builder,
        }
    }

    /// Consume the next character and advance the exact UTF-8 byte cursor.
    pub fn next(&mut self) -> Option<char> {
        let consumed = self.chars.next()?;
        self.last_char_start = self.byte_offset;
        self.byte_offset += consumed.len_utf8() as u32;
        Some(consumed)
    }

    pub fn peek(&mut self) -> Option<&char> {
        self.chars.peek()
    }
    /// WHAT: advance the stream after a successful `peek`, panicking only on an internal
    /// invariant failure.
    ///
    /// WHY: once `peek` has returned `Some`, `next` returning `None` means the stream
    /// invariant is broken, not that user source is malformed.
    pub fn advance_after_peek(&mut self, invariant_message: &'static str) -> char {
        self.next().expect(invariant_message)
    }

    /// Encode the current anchored token range as a source-local span.
    pub fn current_local_span(&mut self) -> Result<LocalSpan, SpanCapacityError> {
        self.local_span_for_bytes(self.start_byte_offset, self.byte_offset)
    }

    /// Encode the current anchored token range as a source-qualified span.
    pub fn current_source_span(&mut self) -> Result<SourceSpan, SpanCapacityError> {
        Ok(SourceSpan::new(self.file_id, self.current_local_span()?))
    }

    /// Encode an exact source-qualified byte range.
    pub fn source_span_for_bytes(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<SourceSpan, SpanCapacityError> {
        Ok(SourceSpan::new(
            self.file_id,
            self.local_span_for_bytes(start, end)?,
        ))
    }

    fn local_span_for_bytes(
        &mut self,
        start: u32,
        end: u32,
    ) -> Result<LocalSpan, SpanCapacityError> {
        let length = end
            .checked_sub(start)
            .expect("token byte cursor moved before its anchored start");
        LocalSpan::exact(start, length, &mut *self.extended_span_builder)
    }

    /// Mint one authored token and encode its exact byte interval.
    pub fn new_token(&mut self, kind: TokenKind) -> Result<Token, SpanCapacityError> {
        let span = self.current_local_span()?;
        self.start_byte_offset = self.byte_offset;
        Ok(Token::new(kind, span))
    }

    /// Anchor the token's byte range at the character already consumed.
    ///
    /// WHY: the lexer reads a token's first character before it can classify the token, and it
    /// skips leading whitespace, comments and discarded template bodies the same way. The
    /// authored token begins at that character's own offset, never at the cursor sitting after
    /// it or at the start of the trivia that preceded it.
    pub fn begin_token_bytes_at_consumed_char(&mut self) {
        self.start_byte_offset = self.last_char_start;
    }

    /// Anchor the token's byte range at the cursor: a zero-width insertion point.
    ///
    /// `Eof` denotes a position rather than authored text, so it must not inherit the byte start
    /// of whatever trivia the lexer skipped to reach the end of the source.
    pub fn begin_token_bytes_at_cursor(&mut self) {
        self.start_byte_offset = self.byte_offset;
    }

    pub fn push_template_mode(&mut self, mode: TokenizeMode) {
        self.template_mode_stack.push(TemplateModeFrame::new(mode));
        self.mode = mode;
    }

    pub fn set_current_template_mode(&mut self, mode: TokenizeMode) {
        // `:` switches the current template from head parsing to body parsing
        // without closing the template nesting level, so mutate the top frame.
        if let Some(current_mode) = self.template_mode_stack.last_mut() {
            current_mode.mode = mode;
            if mode == TokenizeMode::TemplateBody && current_mode.body_mode.is_balanced_mode() {
                // Balanced template-body modes terminate only when square brackets are
                // balanced. The opening `[` that started this template counts as one open.
                current_mode.body_open_square_brackets = 1;
                current_mode.body_closed_square_brackets = 0;
            }
        } else {
            self.template_mode_stack.push(TemplateModeFrame::new(mode));
        }

        self.mode = mode;
    }

    pub fn pop_template_mode(&mut self) {
        // `]` closes exactly one template nesting level. Keep the initial frame so
        // tokenization started in a template mode cannot escape back to normal mode.
        if self.template_mode_stack.len() > 1 {
            self.template_mode_stack.pop();
        }

        self.mode = *self
            .template_mode_stack
            .last()
            .map(|frame| &frame.mode)
            .unwrap_or(&TokenizeMode::Normal);
    }

    pub fn initial_template_close_rejection(&self) -> Option<SourceFileKind> {
        let current_mode = self.template_mode_stack.last()?;

        if self.template_mode_stack.len() != 1 || current_mode.mode != TokenizeMode::TemplateBody {
            return None;
        }

        match current_mode.initial_close_policy {
            InitialTemplateClosePolicy::Allow => None,
            InitialTemplateClosePolicy::RejectOuterClose { source_kind } => Some(source_kind),
        }
    }

    pub fn mark_current_template_body_mode(&mut self, body_mode: TemplateBodyMode) {
        if let Some(current_mode) = self.template_mode_stack.last_mut() {
            current_mode.body_mode = body_mode;
            if current_mode.mode == TokenizeMode::TemplateBody && body_mode.is_balanced_mode() {
                current_mode.body_open_square_brackets = 1;
                current_mode.body_closed_square_brackets = 0;
            }
        }
    }

    pub fn current_template_body_mode(&self) -> TemplateBodyMode {
        self.template_mode_stack
            .last()
            .map(|frame| frame.body_mode)
            .unwrap_or_default()
    }

    pub fn register_template_body_open_square_bracket(&mut self) {
        if let Some(current_mode) = self.template_mode_stack.last_mut()
            && current_mode.body_mode.is_balanced_mode()
        {
            current_mode.body_open_square_brackets =
                current_mode.body_open_square_brackets.saturating_add(1);
        }
    }

    pub fn register_template_body_close_square_bracket(&mut self) {
        if let Some(current_mode) = self.template_mode_stack.last_mut()
            && current_mode.body_mode.is_balanced_mode()
        {
            current_mode.body_closed_square_brackets =
                current_mode.body_closed_square_brackets.saturating_add(1);
        }
    }

    pub fn template_body_next_close_balances_brackets(&self) -> bool {
        let Some(current_mode) = self.template_mode_stack.last() else {
            return false;
        };

        if current_mode.mode != TokenizeMode::TemplateBody
            || !current_mode.body_mode.is_balanced_mode()
        {
            return false;
        }

        current_mode.body_closed_square_brackets.saturating_add(1)
            == current_mode.body_open_square_brackets
    }
}

/// How a diagnostic or source-token shape obtains its user-facing spelling.
///
/// This metadata belongs to the tokenizer taxonomy so source-token stores and diagnostic
/// projections cannot grow independent token-name authorities.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TokenDescriptorPayload {
    Static = 0,
    Symbol = 1,
    StyleDirective = 2,
    StringLiteral = 3,
    NumericLiteral = 4,
    CharLiteral = 5,
    RawStringLiteral = 6,
    BoolLiteral = 7,
    Path = 8,
}

/// Static metadata for one stable token tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenDescriptor {
    text: &'static str,
    payload: TokenDescriptorPayload,
}

impl TokenDescriptor {
    const fn new(text: &'static str, payload: TokenDescriptorPayload) -> Self {
        Self { text, payload }
    }

    pub(crate) const fn text(self) -> &'static str {
        self.text
    }

    pub(crate) const fn payload(self) -> TokenDescriptorPayload {
        self.payload
    }
}

/// Stable compact taxonomy shared by tokenizer shapes and diagnostic projections.
///
/// Every value is explicit in the schema invocation below. It is deliberately independent of
/// `TokenKind` declaration order so adding or reordering source variants cannot change retained
/// diagnostic data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenTag(u16);

/// Fixed source-token shape: one stable tag, reserved/semantic flags and one compact payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenShape {
    pub(crate) tag: TokenTag,
    pub(crate) flags: u16,
    pub(crate) data: u32,
}

const _: () = assert!(std::mem::size_of::<TokenShape>() == 8);

const TOKEN_CLASS_ASSIGNMENT: u16 = 1 << 0;
const TOKEN_CLASS_CONTINUES_EXPRESSION: u16 = 1 << 1;
const TOKEN_CLASS_CAN_END_EXPRESSION: u16 = 1 << 2;
const TOKEN_CLASS_OPERAND_START: u16 = 1 << 3;
const TOKEN_CLASS_KEYWORD: u16 = 1 << 4;
const TOKEN_CLASS_WORD_OPERATOR: u16 = 1 << 5;
const TOKEN_CLASS_LITERAL: u16 = 1 << 6;
const TOKEN_CLASS_BUILTIN_TYPE: u16 = 1 << 7;
const TOKEN_CLASS_DELIMITER: u16 = 1 << 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenSchema {
    tag: TokenTag,
    descriptor: TokenDescriptor,
    allowed_flags: u16,
    classes: u16,
    precedence: Option<u8>,
}

impl TokenSchema {
    pub(crate) const fn tag(self) -> TokenTag {
        self.tag
    }

    pub(crate) const fn descriptor(self) -> TokenDescriptor {
        self.descriptor
    }

    pub(crate) const fn allowed_flags(self) -> u16 {
        self.allowed_flags
    }

    pub(crate) const fn precedence(self) -> Option<u8> {
        self.precedence
    }

    fn has_class(self, class: u16) -> bool {
        self.classes & class != 0
    }
}

macro_rules! token_schema {
    (
        $(
            (
                $pattern:pat,
                $tag_name:ident,
                $raw:literal,
                $text:literal,
                $payload:ident,
                $allowed_flags:expr,
                $classes:expr,
                $precedence:expr
            )
        ),+ $(,)?
    ) => {
        impl TokenTag {
            $(pub(crate) const $tag_name: Self = Self($raw);)+

            pub(crate) const fn raw(self) -> u16 {
                self.0
            }

            pub(crate) const fn from_raw(raw: u16) -> Option<Self> {
                match raw {
                    $($raw => Some(Self::$tag_name),)+
                    _ => None,
                }
            }
            /// Preserve a raw value from a packed boundary even when it is unknown to this
            /// version of the taxonomy. Unknown values render through the descriptor fallback.
            pub(crate) const fn from_raw_unchecked(raw: u16) -> Self {
                Self(raw)
            }


            pub(crate) const fn all() -> &'static [Self] {
                &[$(Self::$tag_name,)+]
            }

            pub(crate) const fn descriptor(self) -> TokenDescriptor {
                match self {
                    $(Self::$tag_name => TokenDescriptor::new(
                        $text,
                        TokenDescriptorPayload::$payload,
                    ),)+
                    _ => TokenDescriptor::new("token", TokenDescriptorPayload::Static),
                }
            }

            pub(crate) const fn schema(self) -> Option<TokenSchema> {
                match self {
                    $(Self::$tag_name => Some(TokenSchema {
                        tag: Self::$tag_name,
                        descriptor: TokenDescriptor::new(
                            $text,
                            TokenDescriptorPayload::$payload,
                        ),
                        allowed_flags: $allowed_flags,
                        classes: $classes,
                        precedence: $precedence,
                    }),)+
                    _ => None,
                }
            }

            pub(crate) const fn allowed_flags(self) -> u16 {
                match self {
                    $(Self::$tag_name => $allowed_flags,)+
                    _ => 0,
                }
            }

            pub(crate) const fn flags_are_valid(self, flags: u16) -> bool {
                flags & !self.allowed_flags() == 0
            }

            pub(crate) fn is_assignment_operator(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_ASSIGNMENT)
                })
            }

            pub(crate) fn continues_expression(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_CONTINUES_EXPRESSION)
                })
            }

            pub(crate) fn can_end_expression(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_CAN_END_EXPRESSION)
                })
            }

            pub(crate) fn is_operand_start(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_OPERAND_START)
                })
            }

            pub(crate) fn is_keyword(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_KEYWORD)
                })
            }

            pub(crate) fn is_word_operator(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_WORD_OPERATOR)
                })
            }

            pub(crate) fn is_literal(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_LITERAL)
                })
            }

            pub(crate) fn is_builtin_type(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_BUILTIN_TYPE)
                })
            }

            pub(crate) fn is_delimiter(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_DELIMITER)
                })
            }

            pub(crate) fn precedence(self) -> Option<u8> {
                self.schema().and_then(TokenSchema::precedence)
            }
        }

        impl TokenKind {
            /// Return the stable taxonomy tag for this token, independent of enum ordinals.
            pub(crate) fn token_tag(&self) -> TokenTag {
                match self {
                    $($pattern => TokenTag::$tag_name,)+
                }
            }
        }
    };
}

const TOKEN_NUMERIC_FLAGS: u16 = 0b11;

#[derive(PartialEq, Debug, Clone)]
pub enum TokenKind {
    // For Compiler
    ModuleStart, // Contains module name space
    Eof,         // End of the file

    /// Module-root API marker for the strict `export:` block; exposes declarations or re-exports
    /// through the module's public export surface. Not a general visibility keyword.
    Export,

    // #
    Hash,

    // Reactive declaration/parameter access marker in ordinary code.
    Reactive,

    /// Function Signatures
    Arrow,

    /// Variable name
    Symbol(StringId),
    // `$md`, `$fresh`, and builder-registered directives inside template heads.
    StyleDirective(StringId),

    // Values
    StringSliceLiteral(StringId),
    Path(PathSyntaxId), // Compile time path resolution; dense handle into FileTokens.path_syntax
    NumericLiteral(NumericLiteralToken),
    CharLiteral(char),
    RawStringLiteral(StringId),
    BoolLiteral(bool),

    // Collections
    OpenCurly,  // {
    CloseCurly, // }

    TypeParameterBracket, // |

    // Structure of Syntax
    Newline,
    End,
    StartTemplateBody,

    // Basic Grammar
    Comma,
    Dot,
    Colon,       // :
    DoubleColon, // ::
    Assign,      // =

    // Reserved receiver / trait syntax
    // `this` is reserved for explicit method receiver parameters.
    // `This` is the trait-local receiver placeholder and remains reserved elsewhere.
    This,
    Must,
    TraitThis,

    // Scope
    OpenParenthesis,  // (
    CloseParenthesis, // )

    As,
    Type,
    Of,

    // Can modify types to become variadic parameters.
    // So any number of values can be passed in
    Variadic, // ..

    // Type Declarations
    Mutable,

    // Datatypes
    DatatypeNone,
    NoneLiteral,
    DatatypeInt,
    DatatypeFloat,
    DatatypeBool,
    DatatypeTrue,
    DatatypeFalse,
    DatatypeString,
    DatatypeChar,

    /// For Errors
    Bang,
    /// For Options
    QuestionMark,

    // Mathematical Operators
    Negative,

    Exponent,
    Multiply,
    Divide,
    Modulus,
    IntDivide,

    ExponentAssign,
    MultiplyAssign,
    DivideAssign,
    ModulusAssign,
    IntDivideAssign,

    Add,
    Subtract,
    AddAssign,
    SubtractAssign,

    // Logical Operators in order of precedence
    Not,
    Is,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,

    And,
    Or,

    // Control Flow
    /// If statements and match statements
    If,
    Else,
    Return,
    /// Attached error-return statement keyword: `return!`.
    ReturnBang,
    Catch,
    Then,
    Checked,
    Async,

    // Explicit builtin cast keyword.
    Cast,
    /// Attached fallible-cast propagation keyword: `cast!`.
    CastBang,

    /// Assertion statement intrinsic.
    ///
    /// WHAT: `assert(condition)` and `assert(condition, "message")` are language-owned
    ///       statement surfaces for runtime invariant checking.
    /// WHY: tokenizing it separately keeps the language-owned statement out of the
    ///      ordinary symbol path, so it cannot be shadowed by user declarations.
    Assert,

    // Loops
    Loop,
    By,
    Break,
    Continue,
    ExclusiveRange, // to

    // Range inclusivity marker
    Ampersand, // &

    // Pattern matching
    FatArrow, // =>
    Wildcard, // _

    // Memory Management
    Copy,

    // Templates
    TemplateClose,
    TemplateHead,

    // Channels
    ChannelSend,    // >>
    ChannelReceive, // <<
    Yield,
}
token_schema! {
    (TokenKind::ModuleStart, MODULE_START, 1, "module start", Static, 0, 0, None),
    (TokenKind::Eof, EOF, 2, "end of file", Static, 0, TOKEN_CLASS_DELIMITER, None),
    (TokenKind::Export, EXPORT, 3, "`export`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Hash, HASH, 4, "`#`", Static, 0, 0, None),
    (TokenKind::Reactive, REACTIVE, 5, "`$`", Static, 0, 0, None),
    (TokenKind::Arrow, ARROW, 6, "`->`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, None),
    (
        TokenKind::Symbol(_),
        SYMBOL,
        7,
        "name",
        Symbol,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::StyleDirective(_),
        STYLE_DIRECTIVE,
        8,
        "style directive",
        StyleDirective,
        0,
        0,
        None
    ),
    (
        TokenKind::StringSliceLiteral(_),
        STRING_SLICE_LITERAL,
        9,
        "string literal",
        StringLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (TokenKind::Path(_), PATH, 10, "path", Path, 0, TOKEN_CLASS_OPERAND_START, None),
    (
        TokenKind::NumericLiteral(_),
        NUMERIC_LITERAL,
        11,
        "numeric literal",
        NumericLiteral,
        TOKEN_NUMERIC_FLAGS,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::CharLiteral(_),
        CHAR_LITERAL,
        12,
        "character literal",
        CharLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::RawStringLiteral(_),
        RAW_STRING_LITERAL,
        13,
        "raw string literal",
        RawStringLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TokenKind::BoolLiteral(_),
        BOOL_LITERAL,
        14,
        "boolean literal",
        BoolLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::OpenCurly,
        OPEN_CURLY,
        15,
        "`{`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::CloseCurly,
        CLOSE_CURLY,
        16,
        "`}`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TokenKind::TypeParameterBracket,
        TYPE_PARAMETER_BRACKET,
        17,
        "`|`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::Newline,
        NEWLINE,
        18,
        "newline",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        TokenKind::End,
        END,
        19,
        "`;`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::StartTemplateBody,
        START_TEMPLATE_BODY,
        20,
        "`:`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        TokenKind::Comma,
        COMMA,
        21,
        "`,`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (TokenKind::Dot, DOT, 22, "`.`", Static, 0, TOKEN_CLASS_DELIMITER, None),
    (
        TokenKind::Colon,
        COLON,
        23,
        "`:`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::DoubleColon,
        DOUBLE_COLON,
        24,
        "`::`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        TokenKind::Assign,
        ASSIGN,
        25,
        "`=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::This,
        THIS,
        26,
        "`this`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (TokenKind::Must, MUST, 27, "`must`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TokenKind::TraitThis,
        TRAIT_THIS,
        28,
        "`This`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (
        TokenKind::OpenParenthesis,
        OPEN_PARENTHESIS,
        29,
        "`(`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::CloseParenthesis,
        CLOSE_PARENTHESIS,
        30,
        "`)`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (TokenKind::As, AS, 31, "`as`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Type, TYPE, 32, "`type`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Of, OF, 33, "`of`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TokenKind::Variadic,
        VARIADIC,
        34,
        "`..`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        TokenKind::Mutable,
        MUTABLE,
        35,
        "`~`",
        Static,
        0,
        TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::DatatypeNone,
        DATATYPE_NONE,
        36,
        "`None` type",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::NoneLiteral,
        NONE_LITERAL,
        37,
        "`none`",
        Static,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::DatatypeInt,
        DATATYPE_INT,
        38,
        "`Int`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::DatatypeFloat,
        DATATYPE_FLOAT,
        39,
        "`Float`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::DatatypeBool,
        DATATYPE_BOOL,
        40,
        "`Bool`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::DatatypeTrue,
        DATATYPE_TRUE,
        41,
        "`True`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::DatatypeFalse,
        DATATYPE_FALSE,
        42,
        "`False`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::DatatypeString,
        DATATYPE_STRING,
        43,
        "`String`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::DatatypeChar,
        DATATYPE_CHAR,
        44,
        "`Char`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        TokenKind::Bang,
        BANG,
        45,
        "`!`",
        Static,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TokenKind::QuestionMark,
        QUESTION_MARK,
        46,
        "`?`",
        Static,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (TokenKind::Negative, NEGATIVE, 47, "unary `-`", Static, 0, 0, Some(6)),
    (TokenKind::Exponent, EXPONENT, 48, "`^`", Static, 0, 0, Some(5)),
    (TokenKind::Multiply, MULTIPLY, 49, "`*`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (TokenKind::Divide, DIVIDE, 50, "`/`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (TokenKind::Modulus, MODULUS, 51, "`%`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (TokenKind::IntDivide, INT_DIVIDE, 52, "`//`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (
        TokenKind::ExponentAssign,
        EXPONENT_ASSIGN,
        53,
        "`^=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::MultiplyAssign,
        MULTIPLY_ASSIGN,
        54,
        "`*=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::DivideAssign,
        DIVIDE_ASSIGN,
        55,
        "`/=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::ModulusAssign,
        MODULUS_ASSIGN,
        56,
        "`%=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::IntDivideAssign,
        INT_DIVIDE_ASSIGN,
        57,
        "`//=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::Add,
        ADD,
        58,
        "`+`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(3)
    ),
    (
        TokenKind::Subtract,
        SUBTRACT,
        59,
        "`-`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(3)
    ),
    (
        TokenKind::AddAssign,
        ADD_ASSIGN,
        60,
        "`+=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::SubtractAssign,
        SUBTRACT_ASSIGN,
        61,
        "`-=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        TokenKind::Not,
        NOT,
        62,
        "`not`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR,
        Some(6)
    ),
    (
        TokenKind::Is,
        IS,
        63,
        "`is`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR | TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        TokenKind::LessThan,
        LESS_THAN,
        64,
        "`<`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        TokenKind::LessThanOrEqual,
        LESS_THAN_OR_EQUAL,
        65,
        "`<=`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        TokenKind::GreaterThan,
        GREATER_THAN,
        66,
        "`>`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        TokenKind::GreaterThanOrEqual,
        GREATER_THAN_OR_EQUAL,
        67,
        "`>=`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        TokenKind::And,
        AND,
        68,
        "`and`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR,
        Some(1)
    ),
    (TokenKind::Or, OR, 69, "`or`", Static, 0, TOKEN_CLASS_WORD_OPERATOR, Some(0)),
    (TokenKind::If, IF, 70, "`if`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Else, ELSE, 71, "`else`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Return, RETURN, 72, "`return`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TokenKind::ReturnBang,
        RETURN_BANG,
        73,
        "`return!`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (TokenKind::Catch, CATCH, 74, "`catch`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Then, THEN, 75, "`then`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Checked, CHECKED, 76, "`checked`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Async, ASYNC, 77, "`async`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Cast, CAST, 78, "`cast`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TokenKind::CastBang,
        CAST_BANG,
        79,
        "`cast!`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (TokenKind::Assert, ASSERT, 80, "`assert`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Loop, LOOP, 81, "`loop`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::By, BY, 82, "`by`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (TokenKind::Break, BREAK, 83, "`break`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TokenKind::Continue,
        CONTINUE,
        84,
        "`continue`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (
        TokenKind::ExclusiveRange,
        EXCLUSIVE_RANGE,
        85,
        "`to`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        Some(6)
    ),
    (TokenKind::Ampersand, AMPERSAND, 86, "`&`", Static, 0, 0, None),
    (
        TokenKind::FatArrow,
        FAT_ARROW,
        87,
        "`=>`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (TokenKind::Wildcard, WILDCARD, 88, "`_`", Static, 0, 0, None),
    (
        TokenKind::Copy,
        COPY,
        89,
        "`copy`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TokenKind::TemplateClose,
        TEMPLATE_CLOSE,
        90,
        "`]`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TokenKind::TemplateHead,
        TEMPLATE_HEAD,
        91,
        "`[`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        TokenKind::ChannelSend,
        CHANNEL_SEND,
        92,
        "`>>`",
        Static,
        0,
        0,
        None
    ),
    (
        TokenKind::ChannelReceive,
        CHANNEL_RECEIVE,
        93,
        "`<<`",
        Static,
        0,
        0,
        None
    ),
    (TokenKind::Yield, YIELD, 94, "`yield`", Static, 0, TOKEN_CLASS_KEYWORD, None),
}

impl TokenShape {
    /// Construct a shape only when its flags are valid for the selected token tag.
    ///
    /// Payload validation is performed by [`Self::from_raw_parts`] and the typed accessors. This
    /// constructor remains a low-level packing primitive for taxonomy tests and source adapters.
    pub(crate) const fn new(tag: TokenTag, flags: u16, data: u32) -> Option<Self> {
        if tag.flags_are_valid(flags) {
            Some(Self { tag, flags, data })
        } else {
            None
        }
    }

    /// Decode a raw shape while rejecting unknown tags, reserved flags and malformed payloads.
    pub(crate) fn from_raw_parts(raw_tag: u16, flags: u16, data: u32) -> Option<Self> {
        let tag = TokenTag::from_raw(raw_tag)?;
        if !tag.flags_are_valid(flags) || !Self::payload_is_valid(tag, flags, data) {
            return None;
        }
        Some(Self { tag, flags, data })
    }

    fn payload_is_valid(tag: TokenTag, flags: u16, data: u32) -> bool {
        match tag.descriptor().payload() {
            TokenDescriptorPayload::Static => flags == 0 && data == 0,
            TokenDescriptorPayload::Path => {
                flags == 0 && PathSyntaxId::try_from_raw(data).is_some()
            }
            TokenDescriptorPayload::NumericLiteral => {
                flags <= 2 && NumericLiteralId::try_from_raw(data).is_some()
            }
            TokenDescriptorPayload::BoolLiteral => flags == 0 && data <= 1,
            TokenDescriptorPayload::CharLiteral => {
                flags == 0 && char::from_u32(data).is_some()
            }
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => flags == 0,
        }
    }

    /// Build the compact shape for a transient `TokenKind` adapter.
    pub(crate) fn from_token_kind(kind: &TokenKind) -> Option<Self> {
        Self::from_token_kind_with_numeric_id(kind, NumericLiteralId::NONE)
    }

    /// Build a shape while supplying the source-owned numeric side-store handle.
    ///
    /// Absent path and numeric handles are malformed payloads and return `None`, so
    /// round-trip validation holds through `from_raw_parts` and the typed accessors.
    pub(crate) fn from_token_kind_with_numeric_id(
        kind: &TokenKind,
        numeric_id: NumericLiteralId,
    ) -> Option<Self> {
        let tag = kind.token_tag();
        let (flags, data) = match kind {
            TokenKind::Symbol(value)
            | TokenKind::StyleDirective(value)
            | TokenKind::StringSliceLiteral(value)
            | TokenKind::RawStringLiteral(value) => (0, value.index()),
            TokenKind::Path(value) => (0, value.raw()),
            TokenKind::NumericLiteral(value) => (numeric_kind_flags(value.kind), numeric_id.raw()),
            TokenKind::CharLiteral(value) => (0, *value as u32),
            TokenKind::BoolLiteral(value) => (0, u32::from(*value)),
            _ => (0, 0),
        };
        Self::from_raw_parts(tag.raw(), flags, data)
    }

    pub(crate) fn numeric_kind(self) -> Option<NumericLiteralKind> {
        if self.tag != TokenTag::NUMERIC_LITERAL || self.flags > 2 {
            return None;
        }
        Some(match self.flags {
            0 => NumericLiteralKind::WholeNumber,
            1 => NumericLiteralKind::DecimalPoint,
            2 => NumericLiteralKind::Exponent,
            _ => unreachable!("numeric flags were checked above"),
        })
    }

    pub(crate) fn numeric_literal_id(self) -> Option<NumericLiteralId> {
        if self.numeric_kind().is_some() {
            NumericLiteralId::try_from_raw(self.data)
        } else {
            None
        }
    }

    pub(crate) fn path_syntax_id(self) -> Option<PathSyntaxId> {
        if self.tag == TokenTag::PATH && self.flags == 0 {
            PathSyntaxId::try_from_raw(self.data)
        } else {
            None
        }
    }

    pub(crate) fn string_id(self) -> Option<StringId> {
        let payload = self.tag.descriptor().payload();
        if matches!(
            payload,
            TokenDescriptorPayload::Symbol
                | TokenDescriptorPayload::StyleDirective
                | TokenDescriptorPayload::StringLiteral
                | TokenDescriptorPayload::RawStringLiteral
        ) && self.flags == 0
        {
            Some(StringId::from_index(self.data))
        } else {
            None
        }
    }

    pub(crate) fn bool_value_checked(self) -> Option<bool> {
        if self.tag == TokenTag::BOOL_LITERAL && self.flags == 0 {
            match self.data {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            }
        } else {
            None
        }
    }

    pub(crate) fn char_value_checked(self) -> Option<char> {
        if self.tag == TokenTag::CHAR_LITERAL && self.flags == 0 {
            char::from_u32(self.data)
        } else {
            None
        }
    }

    pub(crate) const fn tag(self) -> TokenTag {
        self.tag
    }

    pub(crate) const fn flags(self) -> u16 {
        self.flags
    }

    pub(crate) const fn data(self) -> u32 {
        self.data
    }
}

const fn numeric_kind_flags(kind: NumericLiteralKind) -> u16 {
    match kind {
        NumericLiteralKind::WholeNumber => 0,
        NumericLiteralKind::DecimalPoint => 1,
        NumericLiteralKind::Exponent => 2,
    }
}

impl Token {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.kind.remap_string_ids(remap);
    }

    /// Remap every interned payload in place through one fallible string-ID walker.
    pub fn try_remap_string_ids<E>(
        &mut self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<(), E> {
        self.kind.try_remap_string_ids(map)
    }
}

impl TokenKind {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.try_remap_string_ids(&mut |id| {
            Ok::<StringId, std::convert::Infallible>(remap.get(id))
        })
        .expect("token string-ID remapping is infallible");
    }

    /// Remap every interned string payload through one exhaustive, in-place, fallible walker.
    ///
    /// WHAT: the single canonical `TokenKind` string-ID traversal for ordinary remaps, frozen
    ///       body capture and frozen body materialisation.
    /// WHY: every payload-bearing variant is listed explicitly here and mutated in place.
    ///       Adding a new `TokenKind` variant produces one compile error at this owner instead
    ///       of silently retaining a donor `StringId` through a catch-all arm.
    pub fn try_remap_string_ids<E>(
        &mut self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<(), E> {
        match self {
            TokenKind::Symbol(value) => *value = map(*value)?,
            TokenKind::StyleDirective(value) => *value = map(*value)?,
            TokenKind::StringSliceLiteral(value) => *value = map(*value)?,
            TokenKind::RawStringLiteral(value) => *value = map(*value)?,
            TokenKind::NumericLiteral(value) => value.try_remap_string_ids(map)?,
            TokenKind::Path(_) => {}
            TokenKind::CharLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::ModuleStart
            | TokenKind::Eof
            | TokenKind::Export
            | TokenKind::Hash
            | TokenKind::Reactive
            | TokenKind::Arrow
            | TokenKind::OpenCurly
            | TokenKind::CloseCurly
            | TokenKind::TypeParameterBracket
            | TokenKind::Newline
            | TokenKind::End
            | TokenKind::StartTemplateBody
            | TokenKind::Comma
            | TokenKind::Dot
            | TokenKind::Colon
            | TokenKind::DoubleColon
            | TokenKind::Assign
            | TokenKind::This
            | TokenKind::Must
            | TokenKind::TraitThis
            | TokenKind::OpenParenthesis
            | TokenKind::CloseParenthesis
            | TokenKind::As
            | TokenKind::Type
            | TokenKind::Of
            | TokenKind::Variadic
            | TokenKind::Mutable
            | TokenKind::DatatypeNone
            | TokenKind::NoneLiteral
            | TokenKind::DatatypeInt
            | TokenKind::DatatypeFloat
            | TokenKind::DatatypeBool
            | TokenKind::DatatypeTrue
            | TokenKind::DatatypeFalse
            | TokenKind::DatatypeString
            | TokenKind::DatatypeChar
            | TokenKind::Bang
            | TokenKind::QuestionMark
            | TokenKind::Negative
            | TokenKind::Exponent
            | TokenKind::Multiply
            | TokenKind::Divide
            | TokenKind::Modulus
            | TokenKind::IntDivide
            | TokenKind::ExponentAssign
            | TokenKind::MultiplyAssign
            | TokenKind::DivideAssign
            | TokenKind::ModulusAssign
            | TokenKind::IntDivideAssign
            | TokenKind::Add
            | TokenKind::Subtract
            | TokenKind::AddAssign
            | TokenKind::SubtractAssign
            | TokenKind::Not
            | TokenKind::Is
            | TokenKind::LessThan
            | TokenKind::LessThanOrEqual
            | TokenKind::GreaterThan
            | TokenKind::GreaterThanOrEqual
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::Return
            | TokenKind::ReturnBang
            | TokenKind::Catch
            | TokenKind::Then
            | TokenKind::Checked
            | TokenKind::Async
            | TokenKind::Cast
            | TokenKind::CastBang
            | TokenKind::Assert
            | TokenKind::Loop
            | TokenKind::By
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::ExclusiveRange
            | TokenKind::Ampersand
            | TokenKind::FatArrow
            | TokenKind::Wildcard
            | TokenKind::Copy
            | TokenKind::TemplateClose
            | TokenKind::TemplateHead
            | TokenKind::ChannelSend
            | TokenKind::ChannelReceive
            | TokenKind::Yield => {}
        }
        Ok(())
    }

    /// Returns true when this token is a supported assignment operator in statement/write position.
    pub fn is_assignment_operator(&self) -> bool {
        self.token_tag().is_assignment_operator()
    }

    /// Returns true when this token allows a following newline to remain in the same expression.
    pub fn continues_expression(&self) -> bool {
        self.token_tag().continues_expression()
    }

    /// Returns true when this token can be the left operand of a following symbolic operator.
    pub fn can_end_expression(&self) -> bool {
        self.token_tag().can_end_expression()
    }

    /// Returns true when this token may begin a value operand in expression dispatch.
    pub(crate) fn is_operand_start(&self) -> bool {
        self.token_tag().is_operand_start()
    }

    /// Returns true for source words classified as ordinary language keywords.
    pub(crate) fn is_keyword(&self) -> bool {
        self.token_tag().is_keyword()
    }

    /// Returns true for word operators (`not`, `is`, `and`, and `or`).
    pub(crate) fn is_word_operator(&self) -> bool {
        self.token_tag().is_word_operator()
    }

    /// Returns true for value literal tokens, excluding builtin type spellings.
    pub(crate) fn is_literal(&self) -> bool {
        self.token_tag().is_literal()
    }

    /// Returns true for builtin type spellings such as `Int` and `String`.
    pub(crate) fn is_builtin_type(&self) -> bool {
        self.token_tag().is_builtin_type()
    }

    /// Returns true for punctuation and structural syntax delimiters.
    pub(crate) fn is_delimiter(&self) -> bool {
        self.token_tag().is_delimiter()
    }

    /// Return the AST-compatible precedence for operator tokens.
    pub(crate) fn precedence(&self) -> Option<u8> {
        self.token_tag().precedence()
    }
}

#[cfg(test)]
#[path = "tests/tokens_remap_tests.rs"]
mod tokens_remap_tests;

#[cfg(test)]
#[path = "tests/token_taxonomy_tests.rs"]
mod token_taxonomy_tests;
