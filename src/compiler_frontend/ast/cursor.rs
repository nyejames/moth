//! Short-lived cursor used by AST parsers.
//!
//! Canonical parser paths borrow one [`TokenCursor`] over [`SourceTokens`]. Retained parser
//! payloads stay in the canonical source owner and are read through checked [`TokenRef`] views.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::DiagnosticToken;
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::{PathSyntax, PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenPayloadOrigin, TokenRange, TokenRangeError,
    TokenRef, TokenSequenceId, TokenTag, TokenViewError,
};
use std::sync::Arc;

/// Mutable AST parser position over a canonical source-token view.
/// The cursor owns no secondary payload store and never materialises a wide token payload for
/// lookahead. Consumers read tags and typed payloads directly from the checked canonical view.
#[derive(Debug)]
pub(crate) struct AstCursor<'a> {
    cursor: TokenCursor<'a>,
    /// Canonical source ownership retained for bounded parser handoffs.
    canonical_owner: Option<Arc<SourceTokens>>,
    /// Optional frozen donor identity for retained foreign bodies.
    ///
    /// The cursor keeps provenance in the donor `SourceTokens` and translates only payloads that a
    /// requester explicitly reads through the `*_in` accessors.
    payload_origin: Option<TokenPayloadOrigin<'a>>,
    /// Synthetic parser EOF observed at the end of a bounded canonical range.
    ///
    /// WHAT: reports the declaration EOF span without materialising a trailing token.
    /// WHY: canonical initializer cursors share source storage; the EOF is parser data only.
    synthetic_eof: Option<SourceSpan>,
}

impl<'a> AstCursor<'a> {
    fn new_with_owner(cursor: TokenCursor<'a>, canonical_owner: Option<Arc<SourceTokens>>) -> Self {
        Self {
            cursor,
            canonical_owner,
            payload_origin: None,
            synthetic_eof: None,
        }
    }

    /// Construct an owner-retaining canonical cursor directly from one canonical source owner.
    ///
    /// The cursor borrows the owner while retaining an `Arc` clone for later bounded parser
    /// handoffs. Filesystem identity lives in the source database and module symbol maps, never
    /// in the cursor.
    pub(crate) fn from_source_tokens(
        source_tokens: &'a Arc<SourceTokens>,
        range: TokenRange,
    ) -> Result<Self, CompilerError> {
        Self::from_source_tokens_for_handoff(source_tokens, range)
    }

    /// Construct an owner-retaining canonical cursor for a bounded parser handoff.
    ///
    /// The handoff retains only the checked range and source owner; parser facts are read
    /// directly from the cursor view when a consumer asks for them.
    pub(crate) fn from_source_tokens_for_handoff(
        source_tokens: &'a Arc<SourceTokens>,
        range: TokenRange,
    ) -> Result<Self, CompilerError> {
        let canonical = source_tokens.cursor(range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "canonical source range cursor construction failed: {error:?}"
            ))
        })?;
        Ok(Self::new_with_owner(
            canonical,
            Some(Arc::clone(source_tokens)),
        ))
    }

    /// Construct an owner-retaining canonical cursor directly from one segmented sequence.
    ///
    /// Segmented start bodies retain a `TokenSequenceId` rather than a contiguous range;
    /// this shares the same canonical owner without materialising a second payload store.
    pub(crate) fn from_source_sequence(
        source_tokens: &'a Arc<SourceTokens>,
        sequence: TokenSequenceId,
    ) -> Result<Self, CompilerError> {
        let view = source_tokens.token_sequence(sequence).map_err(|error| {
            CompilerError::compiler_error(format!(
                "canonical source sequence cursor construction failed: {error:?}"
            ))
        })?;
        let canonical = view.cursor().map_err(|error| {
            CompilerError::compiler_error(format!(
                "canonical source sequence cursor construction failed: {error:?}"
            ))
        })?;
        Ok(Self::new_with_owner(
            canonical,
            Some(Arc::clone(source_tokens)),
        ))
    }

    pub(crate) fn with_payload_origin(mut self, origin: TokenPayloadOrigin<'a>) -> Self {
        self.payload_origin = Some(origin);
        self
    }
    pub(crate) fn with_synthetic_eof_span(mut self, span: SourceSpan) -> Self {
        self.synthetic_eof = Some(span);
        self
    }

    fn active_start(&self) -> usize {
        self.cursor.parser_window_start()
    }

    /// Exclusive end of the active parser view.
    pub(crate) fn active_end(&self) -> usize {
        self.cursor.parser_length()
    }

    pub(crate) fn current(&self) -> Option<TokenRef<'a>> {
        let token = self.cursor.current()?;
        self.visible_at(self.cursor.parser_position(), token)
    }

    pub(crate) fn previous(&self) -> Option<TokenRef<'a>> {
        let logical_position = self.cursor.parser_position().checked_sub(1)?;
        let token = self.cursor.parser_previous()?;
        self.visible_at(logical_position, token)
    }

    /// Borrow the next token without materialising a wide payload.
    pub(crate) fn peek_next_ref(&self) -> Option<TokenRef<'a>> {
        let position = self.cursor.parser_position();
        let token = if self.cursor.is_segmented() {
            self.cursor.parser_peek_next()
        } else {
            self.cursor.peek_next()
        }?;
        self.visible_at(position.checked_add(1)?, token)
    }

    /// Borrow one token at a parser-coordinate position without materialising a wide payload.
    pub(crate) fn token_ref_at(&self, index: usize) -> Option<TokenRef<'a>> {
        if index < self.active_start() || index >= self.active_end() {
            return None;
        }
        let token = self.cursor.parser_token_at(index)?;
        self.visible_at(index, token)
    }

    /// Borrow one token at an offset from the current parser position.
    pub(crate) fn token_ref_at_offset(&self, offset: usize) -> Option<TokenRef<'a>> {
        self.position()
            .checked_add(offset)
            .and_then(|index| self.token_ref_at(index))
    }

    fn string_id_in(
        &self,
        token: TokenRef<'a>,
        destination: &mut StringTable,
    ) -> Result<Option<StringId>, CompilerError> {
        let Some(id) = token.string_id() else {
            return Ok(None);
        };
        let Some(origin) = self.payload_origin else {
            return Ok(Some(id));
        };
        let spelling = origin.strings.try_resolve(id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "donor string handle {id:?} is outside its issuing frozen table"
            ))
        })?;
        Ok(Some(destination.intern(spelling)))
    }

    /// Return a token string payload at an absolute parser position in the requester's table.
    ///
    /// Tag-only lookups deliberately remain separate: callers that consume a string-shaped
    /// payload must opt into this fallible, origin-aware accessor.
    pub(crate) fn token_string_id_at_in(
        &self,
        index: usize,
        destination: &mut StringTable,
    ) -> Result<Option<StringId>, CompilerError> {
        let Some(token) = self.token_ref_at(index) else {
            return Ok(None);
        };
        if matches!(
            token.tag(),
            TokenTag::SYMBOL
                | TokenTag::STYLE_DIRECTIVE
                | TokenTag::STRING_SLICE_LITERAL
                | TokenTag::RAW_STRING_LITERAL
        ) && token.string_id().is_none()
        {
            return Err(CompilerError::compiler_error(
                "canonical string-shaped token is missing its payload",
            ));
        }
        self.string_id_in(token, destination)
    }

    /// Return the current string payload in the requester table's domain.
    pub(crate) fn current_string_id_in(
        &self,
        destination: &mut StringTable,
    ) -> Result<Option<StringId>, CompilerError> {
        let Some(token) = self.current() else {
            return Ok(None);
        };
        if matches!(
            token.tag(),
            TokenTag::SYMBOL
                | TokenTag::STYLE_DIRECTIVE
                | TokenTag::STRING_SLICE_LITERAL
                | TokenTag::RAW_STRING_LITERAL
        ) && token.string_id().is_none()
        {
            return Err(CompilerError::compiler_error(
                "canonical string-shaped token is missing its payload",
            ));
        }

        self.string_id_in(token, destination)
    }
    /// Project the current token into a durable diagnostic payload in the requester's string
    /// domain. Donor-origin strings and numeric text are translated by spelling; static, path,
    /// character and boolean payloads remain compact tag-local values.
    pub(crate) fn current_diagnostic_token(
        &self,
        destination: &mut StringTable,
    ) -> Result<Option<DiagnosticToken>, TokenViewError> {
        self.current()
            .map(|token| {
                DiagnosticToken::try_from_token_ref_in(
                    token,
                    self.payload_origin.map(|origin| origin.strings),
                    destination,
                )
            })
            .transpose()
    }

    /// Project one token borrowed from this cursor's canonical owner while preserving donor
    /// provenance for payloads that are interpreted by the requester.
    pub(crate) fn diagnostic_token_from_ref(
        &self,
        token: TokenRef<'_>,
        destination: &mut StringTable,
    ) -> Result<DiagnosticToken, TokenViewError> {
        DiagnosticToken::try_from_token_ref_in(
            token,
            self.payload_origin.map(|origin| origin.strings),
            destination,
        )
    }

    /// Return the current numeric payload with text IDs translated only when consumed.
    pub(crate) fn current_numeric_literal_in(
        &self,
        destination: &mut StringTable,
    ) -> Result<Option<NumericLiteralToken>, CompilerError> {
        let Some(token) = self.current() else {
            return Ok(None);
        };
        match self.payload_origin {
            Some(origin) => token
                .numeric_literal_in(origin.strings, destination)
                .map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "donor numeric payload could not be translated: {error:?}"
                    ))
                }),
            None => token
                .numeric_literal()
                .map(|literal| literal.cloned())
                .map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical numeric payload could not be read: {error:?}"
                    ))
                }),
        }
    }

    pub(crate) fn current_path_syntax_id(&self) -> Option<PathSyntaxId> {
        self.current().and_then(TokenRef::path_syntax_id)
    }

    /// Read and validate the current authored path row without cloning its source owner.
    pub(crate) fn current_path_syntax(&self) -> Result<Option<&'a PathSyntax>, CompilerError> {
        match self.current() {
            Some(token) => token.path_syntax().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "canonical path payload could not be read: {error:?}"
                ))
            }),
            None => Ok(None),
        }
    }

    pub(crate) fn advance(&mut self) {
        if self.cursor.parser_position() >= self.active_end() || self.cursor.is_at_end() {
            return;
        }
        self.cursor.advance();
    }

    pub(crate) fn skip_newlines(&mut self) {
        while self.current_tag() == TokenTag::NEWLINE && !self.is_at_end() {
            self.advance();
        }
    }

    pub(crate) fn position(&self) -> usize {
        self.cursor.parser_position()
    }

    /// Exclusive end of what this cursor may parse.
    pub(crate) fn length(&self) -> usize {
        self.active_end()
    }

    pub(crate) fn span_at(&self, index: usize) -> Option<SourceSpan> {
        self.token_ref_at(index).map(TokenRef::source_span)
    }

    pub(crate) fn declaration_cursor(&self) -> Result<DeclarationCursor<'_>, CompilerError> {
        DeclarationCursor::new(self.cursor)
            .map(|declaration| declaration.with_payload_origin(self.payload_origin))
    }

    pub(crate) fn set_position(&mut self, position: usize) -> Result<(), CompilerError> {
        if position < self.active_start() {
            return Err(CompilerError::compiler_error(
                "AST cursor position precedes its active parser window",
            ));
        }
        if position > self.active_end() {
            return Err(CompilerError::compiler_error(
                "AST cursor position exceeds its active parser limit",
            ));
        }
        if self.cursor.is_segmented() {
            self.cursor.set_parser_position(position).map_err(|error| {
                CompilerError::compiler_error(format!(
                    "AST segmented cursor reposition failed: {error:?}"
                ))
            })?;
        } else {
            let position = TokenIndex::try_from_index(position).ok_or_else(|| {
                CompilerError::compiler_error(
                    "AST cursor position exceeds the checked index domain",
                )
            })?;
            self.cursor.set_position(position).map_err(|error| {
                CompilerError::compiler_error(format!("AST cursor reposition failed: {error:?}"))
            })?;
        }
        Ok(())
    }

    /// Narrow the active parser view's end and return the replaced end for restoration.
    pub(crate) fn set_limit(&mut self, end: usize) -> Result<usize, CompilerError> {
        let current = self.position();
        if end < current {
            return Err(CompilerError::compiler_error(
                "AST cursor parser limit precedes its current position",
            ));
        }
        let previous = self.active_end();
        if end > previous {
            return Err(CompilerError::compiler_error(
                "AST cursor parser limit exceeds its active range",
            ));
        }
        let window_start = self.cursor.parser_window_start();
        self.cursor.narrow_parser_window(window_start, end)?;
        Ok(previous)
    }

    /// Restore an end returned by `set_limit`.
    ///
    /// Only a saved end can be restored, so a nested parse can never widen the view it inherited.
    pub(crate) fn restore_limit(&mut self, end: usize) {
        let window_start = self.cursor.parser_window_start();
        self.cursor.restore_parser_window((window_start, end));
    }

    pub(crate) fn is_at_end(&self) -> bool {
        self.position() >= self.active_end() || self.cursor.is_at_end()
    }

    pub(crate) fn nested(&self, range: TokenRange) -> Result<TokenCursor<'a>, TokenRangeError> {
        self.cursor.nested(range)
    }

    /// Create a nested canonical cursor while preserving this cursor's active parser bounds.
    ///
    /// The nested range must fit inside both the current canonical range and any active parser
    /// window/limit. A child built from a bounded parent therefore cannot widen ordinary parser
    /// reads before the window start or after its effective end.
    pub(crate) fn nested_cursor(&self, range: TokenRange) -> Result<Self, TokenRangeError> {
        let active_start = self.active_start();
        let active_end = self.active_end();
        let within_active_bounds = if self.cursor.is_segmented() {
            let start_range = TokenRange::new(range.source(), range.start(), range.start());
            match (
                start_range.and_then(|range| self.cursor.parser_position_at_range_end(range)),
                self.cursor.parser_position_at_range_end(range),
            ) {
                (Some(start), Some(end)) => start >= active_start && end <= active_end,
                _ => true,
            }
        } else {
            range.start().index() >= active_start && range.end().index() <= active_end
        };
        if !within_active_bounds {
            return Err(TokenRangeError::OutOfBounds {
                start: range.start().raw(),
                end: range.end().raw(),
                len: active_end,
            });
        }

        let nested = self.nested(range)?;
        let mut cursor = Self::new_with_owner(nested, self.canonical_owner.clone());
        cursor.payload_origin = self.payload_origin;
        Ok(cursor)
    }

    /// Create a short-lived canonical child over the half-open parser-position window `[start, end)`.
    ///
    /// Contiguous cursors use source-local positions; segmented cursors use dense sequence
    /// positions, so source gaps are never exposed.
    pub(crate) fn subcursor_window(&self, start: usize, end: usize) -> Result<Self, CompilerError> {
        if start > end {
            return Err(CompilerError::compiler_error(
                "AST cursor subcursor window is inverted",
            ));
        }
        if start < self.active_start() || end > self.active_end() {
            return Err(CompilerError::compiler_error(
                "AST cursor subcursor window is outside its active bounds",
            ));
        }

        let mut child = Self::new_with_owner(self.cursor, self.canonical_owner.clone());
        child.payload_origin = self.payload_origin;
        child.synthetic_eof = self.synthetic_eof;
        child.set_position(start)?;
        child.cursor.narrow_parser_window(start, end)?;
        Ok(child)
    }

    pub(crate) fn current_span(&self) -> SourceSpan {
        if let Some(span) = self.current().map(TokenRef::source_span) {
            return span;
        }
        if self.is_at_end()
            && let Some(span) = self.synthetic_eof
        {
            return span;
        }
        SourceSpan::new(self.source_id(), LocalSpan::source_start())
    }

    pub(crate) fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        self.cursor.source_tokens().path_syntax_table()
    }

    pub(crate) fn previous_span(&self) -> Option<SourceSpan> {
        self.previous().map(TokenRef::source_span)
    }

    pub(crate) fn source_id(&self) -> SourceId {
        self.cursor.range().source()
    }

    pub(crate) fn current_tag(&self) -> TokenTag {
        self.current()
            .map(|token| token.tag())
            .unwrap_or(TokenTag::EOF)
    }

    /// Validate the current trusted shape before a tag-only scanner turns it into syntax.
    ///
    /// Tag classification remains allocation-free; this explicit boundary is used only by
    /// scanners whose fallback diagnosis would otherwise hide a malformed payload.
    pub(crate) fn validate_current_payload(&self) -> Result<(), CompilerError> {
        if let Some(token) = self.current() {
            token.validate_payload().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "canonical token payload invariant failed at index {}: {error:?}",
                    token.index().raw()
                ))
            })?;
        }
        Ok(())
    }

    pub(crate) fn peek_next_tag(&self) -> Option<TokenTag> {
        self.peek_next_ref().map(|token| token.tag())
    }

    pub(crate) fn current_postfix_operator_span(&self) -> SourceSpan {
        self.current_span()
    }

    fn visible_at(&self, logical_position: usize, token: TokenRef<'a>) -> Option<TokenRef<'a>> {
        (logical_position >= self.active_start() && logical_position < self.active_end())
            .then_some(token)
    }
}
