//! Lexer stream state and source-token construction entry points.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::numeric_text::store::NumericLiteralStore;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
use std::iter::Peekable;
use std::str::Chars;

use super::schema::{TokenTag, numeric_kind_flags};
use super::storage::{SourceTokensBuilder, TokenEmitError};
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
    // Nested template heads can appear while parsing another template head/body, and parent/child
    // templates can have different style directives.
    pub template_mode_stack: Vec<TemplateModeFrame>,
    /// Path syntax rows built while lexing.
    pub path_syntax: PathSyntaxTable,
    /// Numeric literal records staged during lexing.
    pub numeric_literals: NumericLiteralStore,
    /// Canonical source-token arrays packed in the same forward pass as lexical recognition.
    pub source_tokens_builder: SourceTokensBuilder,
    /// One mutable extended-span builder borrowed from the caller for every token encoded by
    /// this source stream.
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
        Self::with_capacity(source_code, file_id, entry_mode, extended_span_builder, 0)
    }

    pub fn with_capacity(
        source_code: &'a str,
        file_id: SourceId,
        entry_mode: TokenizerEntryMode,
        extended_span_builder: &'a mut ExtendedSpanBuilder,
        capacity: usize,
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
            source_tokens_builder: SourceTokensBuilder::with_capacity(file_id, capacity),
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

    fn emit_payload(
        &mut self,
        tag: TokenTag,
        flags: u16,
        data: u32,
    ) -> Result<TokenTag, TokenEmitError> {
        let span = self.current_local_span().map_err(TokenEmitError::Span)?;
        self.source_tokens_builder
            .push_payload(tag, flags, data, span)
            .map_err(TokenEmitError::Build)?;
        self.start_byte_offset = self.byte_offset;
        Ok(tag)
    }

    pub(crate) fn emit_static(&mut self, tag: TokenTag) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(tag, 0, 0)
    }

    pub(crate) fn emit_symbol(&mut self, value: StringId) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::SYMBOL, 0, value.index())
    }

    pub(crate) fn emit_style_directive(
        &mut self,
        value: StringId,
    ) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::STYLE_DIRECTIVE, 0, value.index())
    }

    pub(crate) fn emit_string_literal(
        &mut self,
        value: StringId,
    ) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::STRING_SLICE_LITERAL, 0, value.index())
    }

    pub(crate) fn emit_raw_string(&mut self, value: StringId) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::RAW_STRING_LITERAL, 0, value.index())
    }

    pub(crate) fn emit_char(&mut self, value: char) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::CHAR_LITERAL, 0, value as u32)
    }

    pub(crate) fn emit_bool(&mut self, value: bool) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::BOOL_LITERAL, 0, u32::from(value))
    }

    pub(crate) fn emit_path(&mut self, root: PathId) -> Result<TokenTag, TokenEmitError> {
        let span = self.current_local_span().map_err(TokenEmitError::Span)?;
        // The token endpoint must be checked before the path row can mutate its dense store.
        self.source_tokens_builder
            .preflight_next_token()
            .map_err(TokenEmitError::Build)?;
        let path_id = self
            .path_syntax
            .try_push_for_source(root, self.file_id, span)
            .map_err(TokenEmitError::Path)?;
        self.source_tokens_builder
            .push_payload(TokenTag::PATH, 0, path_id.raw(), span)
            .map_err(TokenEmitError::Build)?;
        self.start_byte_offset = self.byte_offset;
        Ok(TokenTag::PATH)
    }

    pub(crate) fn emit_numeric(
        &mut self,
        literal: NumericLiteralToken,
    ) -> Result<TokenTag, TokenEmitError> {
        let span = self.current_local_span().map_err(TokenEmitError::Span)?;
        let numeric_id = self
            .source_tokens_builder
            .preflight_numeric()
            .map_err(TokenEmitError::Build)?;
        let actual_id = self
            .numeric_literals
            .try_push_for_source(self.file_id, literal)
            .map_err(TokenEmitError::Numeric)?;
        debug_assert_eq!(actual_id, numeric_id);
        self.source_tokens_builder
            .push_numeric(
                TokenTag::NUMERIC_LITERAL,
                numeric_kind_flags(
                    self.numeric_literals
                        .try_get_for_source(actual_id, self.file_id)
                        .map_err(TokenEmitError::Numeric)?
                        .kind,
                ),
                actual_id,
                span,
            )
            .map_err(TokenEmitError::Build)?;
        self.start_byte_offset = self.byte_offset;
        Ok(TokenTag::NUMERIC_LITERAL)
    }

    pub(crate) fn take_source_tokens_builder(&mut self) -> SourceTokensBuilder {
        std::mem::replace(
            &mut self.source_tokens_builder,
            SourceTokensBuilder::with_capacity(self.file_id, 0),
        )
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
