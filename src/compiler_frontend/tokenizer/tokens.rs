//! Token definitions and source-location primitives for the frontend tokenizer.
//!
//! WHAT: defines token kinds, token records, and the location metadata threaded through parsing.
//! WHY: every frontend stage past lexing depends on one canonical token and location model.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_errors::CompilerError;
pub use crate::compiler_frontend::compiler_messages::source_location::{
    CharPosition, SourceLocation,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
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
    pub location: SourceLocation,
}

impl Token {
    pub fn new(kind: TokenKind, location: SourceLocation) -> Self {
        Self { kind, location }
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

    fn shared(table: PathSyntaxTable) -> Self {
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
    ///
    /// WHAT: the one file-owned path table lifecycle shared by retained token substreams.
    pub path_syntax: FilePathSyntax,
    pub src_path: InternedPath,
    /// Stable source-file identity for this token stream.
    ///
    /// WHAT: carries frontend file identity into downstream parsing stages.
    /// WHY: entry-file detection and diagnostics should not rely on comparing path text.
    pub file_id: Option<FileId>,
    /// Canonical filesystem source path for IO/path-resolution-only logic.
    pub canonical_os_path: Option<PathBuf>,
    // WHAT: Cheap token classification gathered during lexing.
    // WHY: stats travel with the token stream so header preparation can carry them into the
    //      module-wide aggregation without a second token traversal.
    pub(crate) token_stats: TokenStats,
    pub index: usize,
    pub length: usize,
}

impl FileTokens {
    #[cfg(test)]
    pub fn new(src_path: InternedPath, tokens: Vec<Token>) -> FileTokens {
        Self::new_with_identity(src_path, None, None, tokens, PathSyntaxTable::new())
    }

    /// Construct the sole mutable path-table owner for a newly tokenized source file.
    pub fn new_with_identity(
        src_path: InternedPath,
        file_id: Option<FileId>,
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

    /// Construct a stream from an already-frozen table owned by a generated persistent artefact.
    ///
    /// This is deliberately separate from source construction: generated generic materialisation
    /// is the only path that receives an independently captured table rather than the prepared
    /// source's immutable shared table.
    pub(crate) fn new_frozen_with_identity(
        src_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
    ) -> FileTokens {
        Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::shared(path_syntax),
        )
    }

    /// Construct a retained token stream that will receive its table from the completed
    /// prepared-file owner.
    pub fn new_deferred_with_identity(
        src_path: InternedPath,
        file_id: Option<FileId>,
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
        src_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
    ) -> FileTokens {
        FileTokens {
            length: tokens.len(),
            path_syntax,
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
        src_path: InternedPath,
        file_id: Option<FileId>,
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
        src_path: InternedPath,
        file_id: Option<FileId>,
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
    pub fn new_from_slice(
        src_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        source_path_syntax: &FilePathSyntax,
    ) -> Result<FileTokens, CompilerError> {
        Ok(Self::with_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            source_path_syntax.frozen_substream()?,
        ))
    }

    /// Return the canonical path table once the stream has reached a readable lifecycle state.
    pub fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        self.path_syntax.table()
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
            FilePathSyntax::Preparing(path_syntax) | FilePathSyntax::Shared(path_syntax) => {
                FilePathSyntax::Shared(path_syntax)
            }
            FilePathSyntax::Deferred => {
                panic!("test token stream did not retain a file-owned path table to freeze")
            }
        };
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

    pub fn current_location(&self) -> SourceLocation {
        self.tokens[self.index].location.clone()
    }

    /// Return the authored span for a one-character postfix operator at the cursor.
    ///
    /// WHAT: converts the cursor-based token location used for standalone `!` and `?` tokens
    ///       into the authored character span consumed by semantic suffix parsing.
    /// WHY: suffix diagnostics must retain the operator the user wrote even after parsing has
    ///       advanced to the following delimiter. This keeps source context in the resolved
    ///       handling fact without rescanning source text.
    pub fn current_postfix_operator_location(&self) -> SourceLocation {
        let mut location = self.current_location();
        if location.start_pos == location.end_pos {
            location.start_pos.char_column = location.start_pos.char_column.saturating_sub(1);
            location.end_pos.char_column = location.end_pos.char_column.saturating_sub(1);
        }
        location
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

    /// Remap all interned string IDs in this token stream into a merged string table.
    ///
    /// WHAT: updates `src_path` and every token's kind and location after a string-table merge.
    /// WHY: tokenization produces per-file local string IDs that must be rewritten before
    ///      module-wide stages consume the token stream.
    ///
    /// NOTE: `canonical_os_path` is intentionally NOT remapped; it is a filesystem identity
    ///       (`PathBuf`), not an interned string identity.
    // This is wired when file-level frontend outputs are merged before module-wide header
    // aggregation. Keeping it beside token remapping makes the traversal owner explicit.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.remap_token_payload_string_ids(remap);

        // Path tokens carry dense handles. The prepared-file output owns the single path-table
        // remap, so substreams remap only their local token and semantic-path payloads here.
    }

    fn remap_token_payload_string_ids(&mut self, remap: &StringIdRemap) {
        self.src_path.remap_string_ids(remap);

        for token in &mut self.tokens {
            token.remap_string_ids(remap);
        }
    }

    /// Remap a token stream while it still owns its mutable path table.
    pub(crate) fn remap_preparing_string_ids(
        &mut self,
        remap: &StringIdRemap,
    ) -> Result<(), CompilerError> {
        // Validate the mutable lifecycle before changing any token payload. The second access is
        // safe because the first borrow ends before the payload traversal begins.
        self.path_syntax.preparing_table_mut()?;
        self.remap_token_payload_string_ids(remap);
        self.path_syntax
            .preparing_table_mut()?
            .remap_string_ids(remap);
        Ok(())
    }

    /// Rebind this token stream to a new module source identity.
    ///
    /// WHAT: replaces `src_path`, `file_id`, `canonical_os_path`, every top-level token
    ///       location scope, and every path-syntax table row location scope with the
    ///       supplied logical path and file identity.
    /// WHY: Stage 0 tokenizes each `.moth` file once against a filesystem identity. After the
    ///      complete module file set is known, `SourceFileTable` assigns the module logical
    ///      path, deterministic `FileId`, and canonical OS path. Retained tokens must adopt
    ///      that identity so downstream header parsing, diagnostics, and dependency shells see
    ///      the same logical source scope as freshly tokenized files.
    ///
    /// This method does not change path roots or source spans (`start_pos`/`end_pos`). Only
    /// the source-scope identity is rebound, once through the owned path syntax table.
    pub fn rebind_source_identity(
        &mut self,
        logical_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
    ) -> Result<(), CompilerError> {
        // Acquire the unique mutable owner before changing any identity field so an invalid
        // lifecycle state cannot leave a partially rebound source stream behind.
        self.path_syntax.preparing_table_mut()?;
        self.src_path = logical_path.clone();
        self.rebind_file_identity(logical_path, file_id, canonical_os_path);
        self.path_syntax
            .preparing_table_mut()?
            .rebind_source_identity(&self.src_path);
        Ok(())
    }

    /// Rebind file-owned identity while preserving this stream's semantic path.
    ///
    /// Header and detached syntax substreams use `src_path` for declaration paths such as
    /// `module/function`, not only for the owning file. Synthetic discovery therefore needs to
    /// update their locations and file identity without erasing that semantic suffix.
    pub fn rebind_file_identity(
        &mut self,
        logical_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
    ) {
        self.file_id = file_id;
        self.canonical_os_path = canonical_os_path;

        for token in &mut self.tokens {
            token.location.scope = logical_path.clone();
        }

        // The prepared-file owner rebinds the shared path table once after all retained
        // substreams have been assembled. Header streams only update their own token locations.
    }
}

pub struct TokenStream<'a> {
    pub file_path: &'a InternedPath,
    pub chars: Peekable<Chars<'a>>,
    pub position: CharPosition,
    pub start_position: CharPosition,
    pub mode: TokenizeMode,
    // WHAT: Stack of per-template parsing frames.
    //
    // WHY: `]` must restore the exact parent mode for nested templates opened by
    // `[`, and template-body behaviour must stay local to the template that
    // declared its head directives.
    //
    // A single global mode (for example, `TokenizeMode::Codeblock`) is not enough:
    // nested template heads can appear while parsing another template head/body,
    // and parent/child templates can have different style directives. We therefore
    // keep code-specific state on the current template frame and pop it naturally
    // when that template closes.
    pub template_mode_stack: Vec<TemplateModeFrame>,
    /// Path syntax rows built while lexing; moved into `FileTokens` when tokenization
    /// completes.
    pub path_syntax: PathSyntaxTable,
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
        file_path: &'a InternedPath,
        entry_mode: TokenizerEntryMode,
    ) -> Self {
        let mode = entry_mode.initial_tokenize_mode();
        let initial_close_policy = match entry_mode {
            TokenizerEntryMode::SourceFile => InitialTemplateClosePolicy::Allow,
            TokenizerEntryMode::TemplateBody {
                initial_close_policy,
            } => initial_close_policy,
        };

        Self {
            file_path,
            chars: source_code.chars().peekable(),
            position: CharPosition::default(),
            start_position: Default::default(),
            mode,
            template_mode_stack: vec![TemplateModeFrame::initial(mode, initial_close_policy)],
            path_syntax: PathSyntaxTable::new(),
        }
    }

    pub fn next(&mut self) -> Option<char> {
        match self.chars.peek() {
            Some(c) => {
                if *c == '\n' {
                    self.position.line_number += 1;
                    self.position.char_column = 0;
                } else {
                    self.position.char_column += 1;
                }

                self.chars.next()
            }

            None => None,
        }
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

    pub fn new_location(&mut self) -> SourceLocation {
        let start_pos = self.start_position;
        self.update_start_position();
        SourceLocation::new(self.file_path.to_owned(), start_pos, self.position)
    }

    pub fn update_start_position(&mut self) {
        self.start_position = self.position;
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
    Block,
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

impl Token {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.kind.remap_string_ids(remap);
        self.location.remap_string_ids(remap);
    }

    /// Remap every interned payload in place through one fallible string-ID walker.
    pub fn try_remap_string_ids<E>(
        &mut self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<(), E> {
        self.kind.try_remap_string_ids(map)?;
        self.location.try_remap_string_ids(map)
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
            | TokenKind::Block
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
        matches!(
            self,
            TokenKind::Assign
                | TokenKind::AddAssign
                | TokenKind::SubtractAssign
                | TokenKind::MultiplyAssign
                | TokenKind::DivideAssign
                | TokenKind::ModulusAssign
                | TokenKind::ExponentAssign
                | TokenKind::IntDivideAssign
        )
    }

    // For figuring out when to break out of or continue expressions and statements
    pub fn continues_expression(&self) -> bool {
        matches!(
            self,
            // Tokens that allow any number of newlines after or before them without breaking a statement or expression,
            TokenKind::Colon
                | TokenKind::OpenParenthesis
                | TokenKind::TypeParameterBracket
                | TokenKind::Comma
                | TokenKind::End
                | TokenKind::Assign
                | TokenKind::AddAssign
                | TokenKind::SubtractAssign
                | TokenKind::MultiplyAssign
                | TokenKind::DivideAssign
                | TokenKind::ModulusAssign
                | TokenKind::ExponentAssign
                | TokenKind::IntDivideAssign
                | TokenKind::Add
                | TokenKind::Subtract
                | TokenKind::Multiply
                | TokenKind::Divide
                | TokenKind::Modulus
                | TokenKind::IntDivide
                | TokenKind::Arrow
                | TokenKind::Is
                | TokenKind::LessThan
                | TokenKind::LessThanOrEqual
                | TokenKind::GreaterThan
                | TokenKind::GreaterThanOrEqual
        )
    }

    /// Returns true when this token can be the left operand of a following symbolic operator.
    ///
    /// WHAT: gives the tokenizer a small, syntax-only way to classify `-` and spacing-sensitive
    /// operators without depending on AST expression parsing.
    /// WHY: signed numeric literals and binary-operator spacing are lexical/readability rules,
    /// but they still need to know whether the preceding token looked like an operand.
    pub fn can_end_expression(&self) -> bool {
        matches!(
            self,
            TokenKind::Symbol(_)
                | TokenKind::This
                | TokenKind::NumericLiteral(_)
                | TokenKind::StringSliceLiteral(_)
                | TokenKind::RawStringLiteral(_)
                | TokenKind::CharLiteral(_)
                | TokenKind::BoolLiteral(_)
                | TokenKind::NoneLiteral
                | TokenKind::CloseParenthesis
                | TokenKind::CloseCurly
                | TokenKind::TemplateClose
                | TokenKind::Bang
                | TokenKind::QuestionMark
        )
    }
}

#[cfg(test)]
#[path = "tests/tokens_remap_tests.rs"]
mod tokens_remap_tests;
