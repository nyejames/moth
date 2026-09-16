//! Short-lived cursor used by AST parsers.
//!
//! Canonical parser paths use one borrowed [`TokenCursor`] over [`SourceTokens`]. During the H0
//! cutover, unbounded compatibility-only test and synthetic streams use the explicit legacy
//! backing variant below; it owns no second canonical store and is removed with the remaining
//! `FileTokens` adapters in H5.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::tokenizer::tokens::{
    FilePathSyntax, FileTokens, SourceTokens, Token, TokenCursor, TokenIndex, TokenKind,
    TokenRange, TokenRangeError, TokenRef, TokenTag,
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
enum AstCursorBacking<'a> {
    Canonical(TokenCursor<'a>),
    Compatibility(&'a mut FileTokens),
}

/// Mutable AST parser position over a canonical source-token view.
///
/// The compatibility backing is a short-lived migration adapter only. The cached `TokenKind`
/// values are transient parser facts, not source storage; retained syntax must store checked
/// ranges, sequence IDs, payload IDs, or spans instead.
#[derive(Debug)]
pub(crate) struct AstCursor<'a> {
    backing: AstCursorBacking<'a>,
    /// Canonical source ownership retained for bounded parser handoffs.
    ///
    /// Owner-less canonical cursors leave this absent. Cursors built from `FileTokens` clone
    /// the existing owner `Arc` instead of rebuilding it.
    canonical_owner: Option<Arc<SourceTokens>>,
    /// Filesystem identity used only when a `FileTokens`-backed cursor creates an adapter.
    canonical_os_path: Option<PathBuf>,
    limit: Option<usize>,
    current_kind: TokenKind,
    next_kind: Option<TokenKind>,
    previous_kind: Option<TokenKind>,
}

impl<'a> AstCursor<'a> {
    fn new_with_owner(
        cursor: TokenCursor<'a>,
        canonical_owner: Option<Arc<SourceTokens>>,
        canonical_os_path: Option<PathBuf>,
    ) -> Self {
        let mut cursor = Self {
            backing: AstCursorBacking::Canonical(cursor),
            canonical_owner,
            canonical_os_path,
            limit: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        };
        cursor.refresh_facts();
        cursor
    }

    /// Construct a canonical cursor when the stream has checked provenance; otherwise retain the
    /// explicit short-lived compatibility lane for unbounded synthetic/parser fixtures.
    pub(crate) fn from_file_tokens(
        token_stream: &'a mut FileTokens,
    ) -> Result<Self, CompilerError> {
        let canonical_os_path = token_stream.canonical_os_path.clone();
        let has_canonical_owner = token_stream.has_canonical_source_tokens()
            || token_stream.canonical_source_tokens().is_ok();
        if has_canonical_owner {
            let canonical_owner = token_stream.canonical_source_tokens_arc()?;
            let canonical = token_stream.canonical_cursor_from_current()?;
            return Ok(Self::new_with_owner(
                canonical,
                Some(canonical_owner),
                canonical_os_path,
            ));
        }
        let mut cursor = Self {
            backing: AstCursorBacking::Compatibility(token_stream),
            canonical_owner: None,
            canonical_os_path,
            limit: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        };
        cursor.refresh_facts();
        Ok(cursor)
    }
    /// Construct an owner-retaining canonical cursor directly from one canonical source owner.
    ///
    /// Ordinary header/environment lookups share `Arc<SourceTokens>` plus explicit side-map
    /// filesystem identity. The cursor borrows the owner while retaining an `Arc` clone for
    /// later bounded parser handoffs.
    pub(crate) fn from_source_tokens(
        source_tokens: &'a Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        range: TokenRange,
    ) -> Result<Self, CompilerError> {
        let mut cursor =
            Self::from_source_tokens_for_handoff(source_tokens, canonical_os_path, range)?;
        cursor.refresh_facts();
        Ok(cursor)
    }

    /// Construct an owner-retaining canonical cursor for a bounded parser handoff.
    ///
    /// The handoff only needs the checked range and retained owner; it never reads parser facts
    /// from this cursor. Avoiding the initial current/next/previous materialisation matters for
    /// declaration headers, where the same owner is handed to one short-lived initializer
    /// adapter per constant.
    pub(crate) fn from_source_tokens_for_handoff(
        source_tokens: &'a Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        range: TokenRange,
    ) -> Result<Self, CompilerError> {
        let canonical = source_tokens.cursor(range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "canonical source range cursor construction failed: {error:?}"
            ))
        })?;
        Ok(Self {
            backing: AstCursorBacking::Canonical(canonical),
            canonical_owner: Some(Arc::clone(source_tokens)),
            canonical_os_path,
            limit: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        })
    }

    /// Force the compatibility lane over a remapped generic-body adapter.
    ///
    /// WHAT: reads the transient `FileTokens` vector (rebased payload IDs) instead of the
    /// retained donor canonical provenance.
    /// WHY: `new_remapped_bounded_adapter` keeps donor range/sequence metadata whose spans and
    /// tags still match but whose payload IDs are stale; a canonical cursor would silently
    /// ignore the rebased vector. Infallible: the compatibility lane is always available.
    pub(crate) fn from_file_tokens_compatibility(token_stream: &'a mut FileTokens) -> Self {
        let canonical_os_path = token_stream.canonical_os_path.clone();
        let mut cursor = Self {
            backing: AstCursorBacking::Compatibility(token_stream),
            canonical_owner: None,
            canonical_os_path,
            limit: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        };
        cursor.refresh_facts();
        cursor
    }
    /// Build a bounded expression adapter from the cursor's canonical source owner.
    ///
    /// Cursors created from `FileTokens` retain the canonical owner and filesystem identity so
    /// this handoff shares source storage and preserves diagnostics' OS path. A compatibility
    /// cursor delegates to the existing `FileTokens` constructor, including remapped payloads.
    pub(crate) fn new_bounded_expression_substream(
        source_owner: Option<&AstCursor>,
        range: TokenRange,
        declaration_path: crate::compiler_frontend::symbols::path_interner::PathId,
        eof_span: LocalSpan,
    ) -> Result<FileTokens, CompilerError> {
        let source_owner = source_owner.ok_or_else(|| {
            CompilerError::compiler_error("bounded expression has no canonical source owner")
        })?;
        match &source_owner.backing {
            AstCursorBacking::Canonical(_) => {
                let canonical_owner = source_owner.canonical_owner.as_ref().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "canonical cursor has no source owner for bounded expression",
                    )
                })?;
                FileTokens::new_bounded_expression_substream_from_canonical(
                    Arc::clone(canonical_owner),
                    source_owner.canonical_os_path.clone(),
                    range,
                    declaration_path,
                    eof_span,
                )
            }
            AstCursorBacking::Compatibility(stream) => {
                FileTokens::new_bounded_expression_substream(
                    stream,
                    range,
                    declaration_path,
                    eof_span,
                )
            }
        }
    }

    pub(crate) fn current(&self) -> Option<TokenRef<'a>> {
        let AstCursorBacking::Canonical(cursor) = &self.backing else {
            return None;
        };
        let token = cursor.current()?;
        self.visible_at(cursor.parser_position(), token)
    }

    pub(crate) fn previous(&self) -> Option<TokenRef<'a>> {
        let AstCursorBacking::Canonical(cursor) = &self.backing else {
            return None;
        };
        let logical_position = cursor.parser_position().checked_sub(1)?;
        let token = cursor.parser_previous()?;
        self.visible_at(logical_position, token)
    }

    pub(crate) fn advance(&mut self) {
        let limit = self.limit;
        match &mut self.backing {
            AstCursorBacking::Canonical(cursor) => {
                if limit.is_some_and(|limit| cursor.parser_position() >= limit)
                    || cursor.is_at_end()
                {
                    return;
                }
                cursor.advance();
                self.refresh_facts();
            }
            AstCursorBacking::Compatibility(stream) => {
                if limit.is_some_and(|limit| stream.index >= limit)
                    || stream.index >= stream.length
                    || matches!(
                        stream.tokens.get(stream.index).map(|token| &token.kind),
                        Some(TokenKind::Eof)
                    )
                {
                    return;
                }
                stream.index += 1;
            }
        }
    }

    pub(crate) fn skip_newlines(&mut self) {
        while *self.current_token_kind() == TokenKind::Newline && !self.is_at_end() {
            self.advance();
        }
    }

    pub(crate) fn position(&self) -> usize {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.parser_position(),
            AstCursorBacking::Compatibility(stream) => stream.index,
        }
    }

    pub(crate) fn length(&self) -> usize {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.parser_length(),
            AstCursorBacking::Compatibility(stream) => stream.length,
        }
    }

    pub(crate) fn token_at(&self, index: usize) -> Option<Token> {
        if index >= self.length() || self.limit.is_some_and(|limit| index >= limit) {
            return None;
        }
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => {
                let token = if cursor.is_segmented() {
                    cursor.parser_token_at(index)?
                } else {
                    let index = TokenIndex::try_from_index(index)?;
                    cursor.source_tokens().token(index).ok()?
                };
                token
                    .to_token_kind()
                    .ok()
                    .map(|kind| Token::new(kind, token.span()))
            }
            AstCursorBacking::Compatibility(stream) => stream.tokens.get(index).cloned(),
        }
    }

    pub(crate) fn token_kind_at(&self, index: usize) -> Option<TokenKind> {
        self.token_at(index).map(|token| token.kind)
    }
    /// Read a token kind by its canonical source index.
    ///
    /// Parser-facing accessors use dense sequence positions for segmented cursors; range owners
    /// retain raw source indexes and use this narrow bridge when inspecting a checked `TokenRange`.
    pub(crate) fn raw_token_kind_at(&self, index: usize) -> Option<TokenKind> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => {
                let index = TokenIndex::try_from_index(index)?;
                cursor
                    .source_tokens()
                    .token(index)
                    .ok()
                    .and_then(|token| token.to_token_kind().ok())
            }
            AstCursorBacking::Compatibility(stream) => stream
                .compatibility_index_for_source_index(index)
                .and_then(|index| stream.tokens.get(index))
                .map(|token| token.kind.clone()),
        }
    }

    /// Read a token relative to the current parser position.
    ///
    /// The offset is logical-parser relative for both contiguous and segmented canonical cursors;
    /// the cursor resolves that dense position without changing the live parser position.
    pub(crate) fn token_kind_at_offset(&self, offset: usize) -> Option<TokenKind> {
        if self
            .limit
            .is_some_and(|limit| offset >= limit.saturating_sub(self.position()))
        {
            return None;
        }
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => {
                let position = cursor.parser_position();
                let token = cursor.parser_token_at(position.checked_add(offset)?)?;
                token.to_token_kind().ok()
            }
            AstCursorBacking::Compatibility(stream) => stream
                .tokens
                .get(stream.index.checked_add(offset)?)
                .map(|token| token.kind.clone()),
        }
    }

    pub(crate) fn span_at(&self, index: usize) -> Option<SourceSpan> {
        self.token_at(index)
            .map(|token| SourceSpan::new(self.source_id(), token.span))
    }

    pub(crate) fn declaration_cursor(&self) -> Result<DeclarationCursor<'_>, CompilerError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => DeclarationCursor::new(*cursor),
            AstCursorBacking::Compatibility(stream) => DeclarationCursor::from_file_tokens(stream),
        }
    }

    pub(crate) fn set_position(&mut self, position: usize) -> Result<(), CompilerError> {
        if self.limit.is_some_and(|limit| position > limit) {
            return Err(CompilerError::compiler_error(
                "AST cursor position exceeds its active parser limit",
            ));
        }
        match &mut self.backing {
            AstCursorBacking::Canonical(cursor) => {
                if cursor.is_segmented() {
                    cursor.set_parser_position(position).map_err(|error| {
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
                    cursor.set_position(position).map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "AST cursor reposition failed: {error:?}"
                        ))
                    })?;
                }
            }
            AstCursorBacking::Compatibility(stream) => {
                let end = self.limit.unwrap_or(stream.length);
                if position > end || position > stream.length {
                    return Err(CompilerError::compiler_error(
                        "AST compatibility cursor position is outside its active bounds",
                    ));
                }
                stream.index = position;
            }
        }
        self.refresh_facts();
        Ok(())
    }

    pub(crate) fn set_limit(&mut self, end: usize) -> Result<Option<usize>, CompilerError> {
        let current = self.position();
        if end < current {
            return Err(CompilerError::compiler_error(
                "AST cursor parser limit precedes its current position",
            ));
        }
        let max = match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.parser_length(),
            AstCursorBacking::Compatibility(stream) => stream.length,
        };
        if end > max {
            return Err(CompilerError::compiler_error(
                "AST cursor parser limit exceeds its active range",
            ));
        }
        let previous = self.limit;
        self.limit = Some(end);
        self.refresh_facts();
        Ok(previous)
    }

    pub(crate) fn restore_limit(&mut self, limit: Option<usize>) {
        self.limit = limit;
        self.refresh_facts();
    }
    pub(crate) fn is_at_end(&self) -> bool {
        self.limit.is_some_and(|limit| self.position() >= limit)
            || match &self.backing {
                AstCursorBacking::Canonical(cursor) => cursor.is_at_end(),
                AstCursorBacking::Compatibility(stream) => stream.index >= stream.length,
            }
    }

    pub(crate) fn nested(&self, range: TokenRange) -> Result<TokenCursor<'a>, TokenRangeError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.nested(range),
            AstCursorBacking::Compatibility(_) => Err(TokenRangeError::OutOfBounds {
                start: range.start().raw(),
                end: range.end().raw(),
                len: 0,
            }),
        }
    }
    /// Create a nested canonical cursor while preserving this cursor's provenance metadata.
    ///
    /// The nested cursor keeps the raw range view used by range consumers, but owner-retaining
    /// metadata remains available for bounded parser handoffs from the nested parser.
    pub(crate) fn nested_cursor(&self, range: TokenRange) -> Result<Self, TokenRangeError> {
        let nested = self.nested(range)?;
        Ok(Self::new_with_owner(
            nested,
            self.canonical_owner.clone(),
            self.canonical_os_path.clone(),
        ))
    }

    pub(crate) fn current_span(&self) -> SourceSpan {
        match &self.backing {
            AstCursorBacking::Canonical(_) => self
                .current()
                .map(TokenRef::source_span)
                .unwrap_or_else(|| SourceSpan::new(self.source_id(), LocalSpan::source_start())),
            AstCursorBacking::Compatibility(stream) => stream.current_span(),
        }
    }

    pub(crate) fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.source_tokens().path_syntax_table(),
            AstCursorBacking::Compatibility(stream) => stream.path_syntax_table(),
        }
    }

    pub(crate) fn path_syntax_for_substream(&self) -> Result<FilePathSyntax, CompilerError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => Ok(FilePathSyntax::Shared(
                cursor.source_tokens().path_syntax_arc()?,
            )),
            AstCursorBacking::Compatibility(stream) => stream.path_syntax.frozen_substream(),
        }
    }

    pub(crate) fn previous_span(&self) -> Option<SourceSpan> {
        match &self.backing {
            AstCursorBacking::Canonical(_) => self.previous().map(TokenRef::source_span),
            AstCursorBacking::Compatibility(stream) => stream
                .index
                .checked_sub(1)
                .and_then(|index| stream.tokens.get(index))
                .map(|token| SourceSpan::new(stream.file_id, token.span)),
        }
    }

    pub(crate) fn source_id(&self) -> SourceId {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.range().source(),
            AstCursorBacking::Compatibility(stream) => stream.file_id,
        }
    }

    pub(crate) fn current_tag(&self) -> TokenTag {
        self.current_token_kind().token_tag()
    }

    pub(crate) fn peek_next_tag(&self) -> Option<TokenTag> {
        self.peek_next_token().map(TokenKind::token_tag)
    }

    pub(crate) fn current_token(&self) -> Token {
        match &self.backing {
            AstCursorBacking::Canonical(_) => self
                .current()
                .map(|token| Token::new(self.current_kind.clone(), token.span()))
                .unwrap_or_else(Self::eof_token),
            AstCursorBacking::Compatibility(stream) => stream.current_token(),
        }
    }

    pub(crate) fn current_token_kind(&self) -> &TokenKind {
        match &self.backing {
            AstCursorBacking::Canonical(_) => &self.current_kind,
            AstCursorBacking::Compatibility(stream) => {
                if self.limit.is_some_and(|limit| stream.index >= limit) {
                    return &self.current_kind;
                }
                stream
                    .tokens
                    .get(stream.index)
                    .map(|token| &token.kind)
                    .unwrap_or(&self.current_kind)
            }
        }
    }

    pub(crate) fn peek_next_token(&self) -> Option<&TokenKind> {
        match &self.backing {
            AstCursorBacking::Canonical(_) => self.next_kind.as_ref(),
            AstCursorBacking::Compatibility(stream) => {
                let index = stream.index.checked_add(1)?;
                let end = self.limit.unwrap_or(stream.length);
                (index < end)
                    .then(|| stream.tokens.get(index))
                    .flatten()
                    .map(|token| &token.kind)
            }
        }
    }

    pub(crate) fn previous_token(&self) -> Option<&TokenKind> {
        match &self.backing {
            AstCursorBacking::Canonical(_) => self.previous_kind.as_ref(),
            AstCursorBacking::Compatibility(stream) => stream
                .index
                .checked_sub(1)
                .filter(|index| self.limit.is_none_or(|limit| *index < limit))
                .and_then(|index| stream.tokens.get(index))
                .map(|token| &token.kind),
        }
    }

    pub(crate) fn current_postfix_operator_span(&self) -> SourceSpan {
        self.current_span()
    }

    fn visible_at(&self, logical_position: usize, token: TokenRef<'a>) -> Option<TokenRef<'a>> {
        self.limit
            .is_none_or(|limit| logical_position < limit)
            .then_some(token)
    }

    fn previous_from_cursor(cursor: TokenCursor<'a>, limit: Option<usize>) -> Option<TokenRef<'a>> {
        let current = cursor.current();
        let range = cursor.range();
        let position = current
            .map(TokenRef::index)
            .unwrap_or_else(|| cursor.position());
        let previous = TokenIndex::try_from_raw(position.raw().checked_sub(1)?)?;
        if previous < range.start() {
            return None;
        }
        cursor
            .source_tokens()
            .token(previous)
            .ok()
            .filter(|token| limit.is_none_or(|limit| token.index().index() < limit))
    }

    fn refresh_facts(&mut self) {
        let AstCursorBacking::Canonical(cursor) = &self.backing else {
            // Compatibility cursors expose the already-materialised parser vector directly from
            // their accessors. Re-cloning three TokenKinds on every advance would erase the
            // compatibility lane's purpose.
            return;
        };

        let position = cursor.parser_position();
        let current = cursor
            .current()
            .and_then(|token| self.visible_at(position, token))
            .and_then(|token| token.to_token_kind().ok());

        let next_position = position.checked_add(1);
        let next_token = if cursor.is_segmented() {
            cursor.parser_peek_next()
        } else {
            cursor.peek_next()
        };
        let next = next_position
            .and_then(|position| next_token.and_then(|token| self.visible_at(position, token)))
            .and_then(|token| token.to_token_kind().ok());

        let previous = if cursor.is_segmented() {
            position
                .checked_sub(1)
                .and_then(|position| {
                    cursor
                        .parser_previous()
                        .and_then(|token| self.visible_at(position, token))
                })
                .and_then(|token| token.to_token_kind().ok())
        } else {
            Self::previous_from_cursor(*cursor, self.limit)
                .and_then(|token| token.to_token_kind().ok())
        };

        self.current_kind = current.unwrap_or(TokenKind::Eof);
        self.next_kind = next;
        self.previous_kind = previous;
    }

    fn eof_token() -> Token {
        Token::new(TokenKind::Eof, LocalSpan::source_start())
    }
}
