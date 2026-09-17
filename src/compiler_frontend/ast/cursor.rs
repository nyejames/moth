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
    TokenRange, TokenRangeError, TokenRef, TokenSequenceId, TokenTag,
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
enum AstCursorBacking<'a> {
    Canonical(TokenCursor<'a>),
    Compatibility(&'a mut FileTokens),
    /// Owned temporary compatibility for an explicit synthetic/remapped override.
    ///
    /// WHAT: holds the provided `FileTokens` without borrowing the caller.
    /// WHY: declaration `initializer_override` content is synthetic and has no canonical
    /// range; it stays on the explicit compatibility lane and never widens to canonical paths.
    OwnedCompatibility(FileTokens),
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
    /// Synthetic parser EOF observed at the end of a bounded canonical range.
    ///
    /// WHAT: reports the declaration EOF span without materialising a trailing token.
    /// WHY: canonical initializer cursors share source storage; the EOF is parser data only.
    synthetic_eof: Option<SourceSpan>,
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
            synthetic_eof: None,
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
            synthetic_eof: None,
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
            synthetic_eof: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        })
    }
    /// Construct an owner-retaining canonical cursor directly from one segmented sequence.
    ///
    /// Segmented start bodies retain a `TokenSequenceId` rather than a contiguous range;
    /// this shares the same canonical owner without materialising a `FileTokens` vector.
    pub(crate) fn from_source_sequence(
        source_tokens: &'a Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
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
        let mut cursor = Self {
            backing: AstCursorBacking::Canonical(canonical),
            canonical_owner: Some(Arc::clone(source_tokens)),
            canonical_os_path,
            limit: None,
            synthetic_eof: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        };
        cursor.refresh_facts();
        Ok(cursor)
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
            synthetic_eof: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        };
        cursor.refresh_facts();
        cursor
    }
    pub(crate) fn from_owned_file_tokens_compatibility(token_stream: FileTokens) -> Self {
        let canonical_os_path = token_stream.canonical_os_path.clone();
        let mut cursor = Self {
            backing: AstCursorBacking::OwnedCompatibility(token_stream),
            canonical_owner: None,
            canonical_os_path,
            limit: None,
            synthetic_eof: None,
            current_kind: TokenKind::Eof,
            next_kind: None,
            previous_kind: None,
        };
        cursor.refresh_facts();
        cursor
    }
    /// Build an owned compatibility subcursor for a non-canonical parser stream.
    ///
    /// Canonical callers must use `nested_cursor`; this lane is only for remapped or synthetic
    /// streams whose payload IDs cannot be interpreted through the donor canonical owner.
    pub(crate) fn bounded_compatibility_expression_cursor(
        &self,
        range: TokenRange,
        declaration_path: crate::compiler_frontend::symbols::path_interner::PathId,
        eof_span: LocalSpan,
    ) -> Result<Option<Self>, CompilerError> {
        let Some(stream) = self.compatibility_stream() else {
            return Ok(None);
        };
        let bounded = FileTokens::new_bounded_expression_substream(
            stream,
            range,
            declaration_path,
            eof_span,
        )?;
        Ok(Some(Self::from_owned_file_tokens_compatibility(bounded)))
    }

    pub(crate) fn with_synthetic_eof_span(mut self, span: SourceSpan) -> Self {
        self.synthetic_eof = Some(span);
        self.refresh_facts();
        self
    }

    fn compatibility_stream(&self) -> Option<&FileTokens> {
        match &self.backing {
            AstCursorBacking::Compatibility(stream) => Some(stream),
            AstCursorBacking::OwnedCompatibility(stream) => Some(stream),
            AstCursorBacking::Canonical(_) => None,
        }
    }

    fn compatibility_stream_mut(&mut self) -> Option<&mut FileTokens> {
        match &mut self.backing {
            AstCursorBacking::Compatibility(stream) => Some(stream),
            AstCursorBacking::OwnedCompatibility(stream) => Some(stream),
            AstCursorBacking::Canonical(_) => None,
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
            _ => {
                let stream = self
                    .compatibility_stream_mut()
                    .expect("non-canonical AST cursor must have compatibility backing");
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
            _ => {
                self.compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing")
                    .index
            }
        }
    }
    pub(crate) fn length(&self) -> usize {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.parser_length(),
            _ => {
                self.compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing")
                    .length
            }
        }
    }

    pub(crate) fn token_at(&self, index: usize) -> Option<Token> {
        if index >= self.length() || self.limit.is_some_and(|limit| index >= limit) {
            return None;
        }
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => {
                let token = cursor.parser_token_at(index)?;
                token
                    .to_token_kind()
                    .ok()
                    .map(|kind| Token::new(kind, token.span()))
            }
            _ => self
                .compatibility_stream()
                .and_then(|stream| stream.tokens.get(index).cloned()),
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
            _ => {
                let stream = self.compatibility_stream()?;
                stream
                    .compatibility_index_for_source_index(index)
                    .and_then(|index| stream.tokens.get(index))
                    .map(|token| token.kind.clone())
            }
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
            _ => {
                let stream = self.compatibility_stream()?;
                stream
                    .tokens
                    .get(stream.index.checked_add(offset)?)
                    .map(|token| token.kind.clone())
            }
        }
    }

    pub(crate) fn span_at(&self, index: usize) -> Option<SourceSpan> {
        self.token_at(index)
            .map(|token| SourceSpan::new(self.source_id(), token.span))
    }

    pub(crate) fn declaration_cursor(&self) -> Result<DeclarationCursor<'_>, CompilerError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => DeclarationCursor::new(*cursor),
            _ => DeclarationCursor::from_file_tokens(
                self.compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing"),
            ),
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
            _ => {
                let end = self.limit.unwrap_or_else(|| {
                    self.compatibility_stream()
                        .expect("non-canonical AST cursor must have compatibility backing")
                        .length
                });
                let stream = self
                    .compatibility_stream_mut()
                    .expect("non-canonical AST cursor must have compatibility backing");
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
            _ => {
                self.compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing")
                    .length
            }
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
                _ => {
                    let stream = self
                        .compatibility_stream()
                        .expect("non-canonical AST cursor must have compatibility backing");
                    stream.index >= stream.length
                }
            }
    }

    pub(crate) fn nested(&self, range: TokenRange) -> Result<TokenCursor<'a>, TokenRangeError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.nested(range),
            _ => Err(TokenRangeError::OutOfBounds {
                start: range.start().raw(),
                end: range.end().raw(),
                len: 0,
            }),
        }
    }
    /// Create a nested canonical cursor while preserving this cursor's provenance metadata and
    /// active parser limit.
    ///
    /// The nested range must fit inside both the current canonical range and any active parser
    /// limit. A child built from a bounded parent therefore cannot widen ordinary parser reads.
    pub(crate) fn nested_cursor(&self, range: TokenRange) -> Result<Self, TokenRangeError> {
        if let Some(limit) = self.limit {
            if let AstCursorBacking::Canonical(cursor) = &self.backing {
                if cursor
                    .parser_position_at_range_end(range)
                    .is_none_or(|range_end| range_end > limit)
                {
                    return Err(TokenRangeError::OutOfBounds {
                        start: range.start().raw(),
                        end: range.end().raw(),
                        len: limit,
                    });
                }
            }
        }
        let nested = self.nested(range)?;
        let nested_limit = match (&self.backing, self.limit) {
            (AstCursorBacking::Canonical(cursor), Some(_)) if cursor.is_segmented() => {
                Some(range.end().index())
            }
            (_, limit) => limit,
        };
        let mut cursor = Self::new_with_owner(
            nested,
            self.canonical_owner.clone(),
            self.canonical_os_path.clone(),
        );
        cursor.limit = nested_limit;
        cursor.refresh_facts();
        Ok(cursor)
    }

    pub(crate) fn current_span(&self) -> SourceSpan {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => {
                if let Some(span) = self.current().map(TokenRef::source_span) {
                    return span;
                }
                if cursor.is_at_end() {
                    if let Some(span) = self.synthetic_eof {
                        return span;
                    }
                }
                SourceSpan::new(self.source_id(), LocalSpan::source_start())
            }
            _ => self
                .compatibility_stream()
                .expect("non-canonical AST cursor must have compatibility backing")
                .current_span(),
        }
    }

    pub(crate) fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.source_tokens().path_syntax_table(),
            _ => self
                .compatibility_stream()
                .expect("non-canonical AST cursor must have compatibility backing")
                .path_syntax_table(),
        }
    }

    pub(crate) fn path_syntax_for_substream(&self) -> Result<FilePathSyntax, CompilerError> {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => Ok(FilePathSyntax::Shared(
                cursor.source_tokens().path_syntax_arc()?,
            )),
            _ => self
                .compatibility_stream()
                .expect("non-canonical AST cursor must have compatibility backing")
                .path_syntax
                .frozen_substream(),
        }
    }

    pub(crate) fn previous_span(&self) -> Option<SourceSpan> {
        match &self.backing {
            AstCursorBacking::Canonical(_) => self.previous().map(TokenRef::source_span),
            _ => {
                let stream = self
                    .compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing");
                stream
                    .index
                    .checked_sub(1)
                    .and_then(|index| stream.tokens.get(index))
                    .map(|token| SourceSpan::new(stream.file_id, token.span))
            }
        }
    }

    pub(crate) fn source_id(&self) -> SourceId {
        match &self.backing {
            AstCursorBacking::Canonical(cursor) => cursor.range().source(),
            _ => {
                self.compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing")
                    .file_id
            }
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
            AstCursorBacking::Canonical(cursor) => self
                .current()
                .map(|token| Token::new(self.current_kind.clone(), token.span()))
                .unwrap_or_else(|| {
                    let span = cursor
                        .is_at_end()
                        .then(|| self.synthetic_eof)
                        .flatten()
                        .map(SourceSpan::local)
                        .unwrap_or_else(LocalSpan::source_start);
                    Token::new(TokenKind::Eof, span)
                }),
            _ => self
                .compatibility_stream()
                .expect("non-canonical AST cursor must have compatibility backing")
                .current_token(),
        }
    }

    pub(crate) fn current_token_kind(&self) -> &TokenKind {
        match &self.backing {
            AstCursorBacking::Canonical(_) => &self.current_kind,
            _ => {
                let stream = self
                    .compatibility_stream()
                    .expect("non-canonical AST cursor must have compatibility backing");
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
            _ => {
                let stream = self.compatibility_stream()?;
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
            _ => {
                let stream = self.compatibility_stream()?;
                stream
                    .index
                    .checked_sub(1)
                    .filter(|index| self.limit.is_none_or(|limit| *index < limit))
                    .and_then(|index| stream.tokens.get(index))
                    .map(|token| &token.kind)
            }
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
}
