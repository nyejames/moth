//! Token definitions and source-location primitives for the frontend tokenizer.
//!
//! WHAT: defines token kinds, token records, and the location metadata threaded through parsing.
//! WHY: every frontend stage past lexing depends on one canonical token and location model.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::arena::TokenStats;
pub use crate::compiler_frontend::compiler_messages::source_location::{
    CharPosition, SourceLocation,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::token_log;
use std::iter::Peekable;
use std::path::PathBuf;
use std::str::Chars;

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

/// WHAT: One path entry produced by the path tokenizer, with optional per-entry alias.
/// WHY: Grouped import syntax `import @base { a as x, b }` needs each expanded path to
///      carry its own alias and source location. Storing alias metadata in the token
///      payload avoids reparsing and keeps alias data attached to the entry that
///      produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct PathTokenItem {
    pub path: InternedPath,
    pub alias: Option<StringId>,
    pub path_location: SourceLocation,
    pub alias_location: Option<SourceLocation>,
    /// True when this entry came from grouped path syntax, even if the group
    /// expanded to only one path.
    pub from_grouped: bool,
}

impl PathTokenItem {
    /// Remap all interned string IDs in this path token item into a merged string table.
    ///
    /// WHAT: updates `path`, `alias`, and both locations after a string-table merge.
    /// WHY: path token items carry `InternedPath` and `SourceLocation` data that must stay
    ///      valid when per-file local tables are merged into the module/global table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.path.remap_string_ids(remap);

        if let Some(alias) = &mut self.alias {
            *alias = remap.get(*alias);
        }

        self.path_location.remap_string_ids(remap);

        if let Some(alias_location) = &mut self.alias_location {
            alias_location.remap_string_ids(remap);
        }
    }
}

/// WHAT: Extract bare paths from a slice of path token items.
/// WHY: Non-import consumers (template heads, project config) only need the path data.
pub fn path_token_paths(items: &[PathTokenItem]) -> Vec<InternedPath> {
    items.iter().map(|item| item.path.clone()).collect()
}

#[derive(Clone, Debug)]
pub struct FileTokens {
    pub tokens: Vec<Token>,
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
    pub fn new(src_path: InternedPath, tokens: Vec<Token>) -> FileTokens {
        Self::new_with_identity(src_path, None, None, tokens)
    }

    pub fn new_with_file_id(
        src_path: InternedPath,
        file_id: Option<FileId>,
        tokens: Vec<Token>,
    ) -> FileTokens {
        Self::new_with_identity(src_path, file_id, None, tokens)
    }

    pub fn new_with_identity(
        src_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
    ) -> FileTokens {
        FileTokens {
            length: tokens.len(),
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            token_stats: TokenStats::default(),
            index: 0,
        }
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
        self.src_path.remap_string_ids(remap);

        for token in &mut self.tokens {
            token.remap_string_ids(remap);
        }
    }

    /// Rebind this token stream to a new module source identity.
    ///
    /// WHAT: replaces `src_path`, `file_id`, `canonical_os_path`, every top-level token
    ///       location scope, and every `PathTokenItem` path/alias location scope with the
    ///       supplied logical path and file identity.
    /// WHY: Stage 0 tokenizes each `.moth` file once against a filesystem identity. After the
    ///      complete module file set is known, `SourceFileTable` assigns the module logical
    ///      path, deterministic `FileId`, and canonical OS path. Retained tokens must adopt
    ///      that identity so downstream header parsing, diagnostics, and import shells see the
    ///      same logical source scope as freshly tokenized files.
    ///
    /// This method does not change import path payloads (`PathTokenItem.path`) or source spans
    /// (`start_pos`/`end_pos`). Only the source-scope identity is rebound.
    pub fn rebind_source_identity(
        &mut self,
        logical_path: InternedPath,
        file_id: Option<FileId>,
        canonical_os_path: Option<PathBuf>,
    ) {
        self.src_path = logical_path.clone();
        self.file_id = file_id;
        self.canonical_os_path = canonical_os_path;

        for token in &mut self.tokens {
            token.location.scope = logical_path.clone();

            if let TokenKind::Path(items) = &mut token.kind {
                for item in items {
                    item.path_location.scope = logical_path.clone();
                    if let Some(alias_location) = &mut item.alias_location {
                        alias_location.scope = logical_path.clone();
                    }
                }
            }
        }
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

    // Module Import
    /// For Wasm files or host environment - importing from a different module or the host
    Import,
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
    Path(Vec<PathTokenItem>), // Compile time path resolution
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
    /// Remap all interned string IDs in this token into a merged string table.
    ///
    /// WHAT: updates the token kind and source location after a string-table merge.
    /// WHY: tokens carry `StringId` payloads and `SourceLocation` scopes that must stay
    ///      valid when per-file local tables are merged into the module/global table.
    // This is called by token-stream remapping once file-level frontend outputs are merged.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.kind.remap_string_ids(remap);
        self.location.remap_string_ids(remap);
    }
}

impl TokenKind {
    /// Remap all interned string IDs in this token kind into a merged string table.
    ///
    /// WHAT: updates `Symbol`, `StyleDirective`, string literals, raw string literals,
    ///       and `Path` item payloads after a string-table merge.
    /// WHY: token kinds are the primary carriers of interned string identity from the
    ///      tokenizer; remapping them explicitly keeps string-table ownership local to
    ///      the tokenizer module instead of leaking into diagnostics or downstream stages.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            TokenKind::Symbol(id)
            | TokenKind::StyleDirective(id)
            | TokenKind::StringSliceLiteral(id)
            | TokenKind::RawStringLiteral(id) => {
                *id = remap.get(*id);
            }

            TokenKind::NumericLiteral(token) => {
                token.remap_string_ids(remap);
            }

            TokenKind::Path(items) => {
                for item in items {
                    item.remap_string_ids(remap);
                }
            }

            _ => {}
        }
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
