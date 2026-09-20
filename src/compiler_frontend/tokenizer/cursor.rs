//! Borrowed token references and bounded contiguous or segmented cursors.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::{PathSyntax, PathSyntaxId};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{
    StringId, StringTable, StringTableResolver,
};

use super::schema::{TokenDescriptorPayload, TokenShape, TokenTag};
use super::storage::{
    SourceTokens, TokenIndex, TokenRange, TokenRangeError, TokenSequenceError, TokenSequenceRange,
    TokenSequenceView, TokenViewError,
};
/// A borrowed source token with typed, non-cloning payload views.
#[derive(Clone, Copy, Debug)]
pub struct TokenRef<'a> {
    pub(super) tokens: &'a SourceTokens,
    pub(super) index: TokenIndex,
}

impl<'a> TokenRef<'a> {
    pub const fn index(self) -> TokenIndex {
        self.index
    }

    pub const fn source(self) -> SourceId {
        self.tokens.source
    }

    pub fn shape(self) -> TokenShape {
        self.tokens.shapes[self.index.index()]
    }

    pub fn span(self) -> LocalSpan {
        self.tokens.spans[self.index.index()]
    }

    pub fn source_span(self) -> SourceSpan {
        SourceSpan::new(self.source(), self.span())
    }

    pub fn numeric_literal(self) -> Result<Option<&'a NumericLiteralToken>, TokenViewError> {
        let Some(id) = self.shape().numeric_literal_id() else {
            return Ok(None);
        };
        self.tokens
            .numeric_literals
            .try_get_for_source(id, self.source())
            .map(Some)
            .map_err(|_| TokenViewError::MalformedNumericHandle)
    }

    /// Read one authored path row through the source-owned table without cloning it.
    ///
    /// The returned row keeps the donor source span and semantic path identity. A requester that
    /// needs a different path domain must translate that row at its use boundary; this view never
    /// copies or rewrites the canonical owner.
    pub(crate) fn path_syntax(self) -> Result<Option<&'a PathSyntax>, TokenViewError> {
        let Some(id) = self.shape().path_syntax_id() else {
            return Ok(None);
        };
        let table = self
            .tokens
            .path_syntax
            .as_deref()
            .ok_or(TokenViewError::MissingPathTable)?;
        table
            .try_path_for_token(id, self.source_span())
            .map(Some)
            .map_err(|_| TokenViewError::MalformedPathHandle)
    }

    /// Resolve one string-shaped payload through the table that issued its handle.
    pub(crate) fn string_spelling<S: StringTableResolver + ?Sized>(
        self,
        strings: &S,
    ) -> Result<Option<&str>, TokenViewError> {
        let Some(id) = self.string_id() else {
            return Ok(None);
        };
        strings
            .try_resolve(id)
            .map(Some)
            .ok_or(TokenViewError::MalformedStringHandle)
    }

    /// Re-intern one numeric payload only when a requester consumes it.
    pub(crate) fn numeric_literal_in<S: StringTableResolver + ?Sized>(
        self,
        source_strings: &S,
        destination_strings: &mut StringTable,
    ) -> Result<Option<NumericLiteralToken>, TokenViewError> {
        let Some(literal) = self.numeric_literal()? else {
            return Ok(None);
        };
        let mut literal = literal.clone();
        literal
            .try_remap_string_ids(&mut |id| {
                let spelling = source_strings
                    .try_resolve(id)
                    .ok_or(TokenViewError::MalformedStringHandle)?;
                Ok(destination_strings.intern(spelling))
            })
            .map_err(|error| error)?;
        Ok(Some(literal))
    }

    pub(crate) fn string_id(self) -> Option<StringId> {
        self.shape().string_id()
    }

    /// Stable tag for this token without cloning cold payloads.
    pub(crate) fn tag(self) -> TokenTag {
        self.shape().tag()
    }

    /// Dense path handle for this token, if it carries one.
    ///
    /// Header-stage callers resolve the row through the canonical owner's path table.
    pub(crate) fn path_syntax_id(self) -> Option<PathSyntaxId> {
        self.shape().path_syntax_id()
    }

    pub fn bool_value(self) -> Option<bool> {
        self.shape().bool_value_checked()
    }

    pub fn char_value(self) -> Option<char> {
        self.shape().char_value_checked()
    }

    pub fn is_eof(self) -> bool {
        self.shape().tag() == TokenTag::EOF
    }
    /// Validate the payload referenced by this canonical shape without materialising a legacy
    /// token value. Payload failures stay on the infrastructure lane for parser scanners that
    /// otherwise need only the stable tag.
    pub(crate) fn validate_payload(self) -> Result<(), TokenViewError> {
        match self.shape().tag().descriptor().payload() {
            TokenDescriptorPayload::Static => {
                if self.shape().flags() == 0 && self.shape().data() == 0 {
                    Ok(())
                } else {
                    Err(TokenViewError::MalformedNumericHandle)
                }
            }
            TokenDescriptorPayload::Path => self
                .path_syntax()?
                .map(|_| ())
                .ok_or(TokenViewError::MalformedPathHandle),
            TokenDescriptorPayload::NumericLiteral => self
                .numeric_literal()?
                .map(|_| ())
                .ok_or(TokenViewError::MalformedNumericHandle),
            TokenDescriptorPayload::BoolLiteral => self
                .bool_value()
                .map(|_| ())
                .ok_or(TokenViewError::MalformedNumericHandle),
            TokenDescriptorPayload::CharLiteral => self
                .char_value()
                .map(|_| ())
                .ok_or(TokenViewError::MalformedNumericHandle),
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => self
                .string_id()
                .map(|_| ())
                .ok_or(TokenViewError::MalformedStringHandle),
        }
    }
}

/// A short-lived cursor over one validated contiguous or segmented token view.
#[derive(Clone, Copy, Debug)]
pub struct TokenCursor<'a> {
    tokens: &'a SourceTokens,
    bounds: TokenCursorBounds<'a>,
    segment_index: usize,
    next: TokenIndex,
    /// Cached dense parser position and total length for segmented cursors.
    ///
    /// Contiguous cursors leave these fields unused so their hot advance path keeps only the
    /// canonical source position and active range.
    logical_position: usize,
    logical_length: usize,
    /// Cached adjacent non-empty segments for segmented traversal.
    segment_start_position: usize,
    previous_segment_index: Option<usize>,
    next_segment_index: Option<usize>,
    /// Active parser view in parser coordinates, as a half-open `[window_start, window_end)`.
    ///
    /// WHAT: bounds every parser-facing read and move. Contiguous cursors use absolute source
    /// indexes; segmented cursors use dense logical positions.
    /// WHY: a bounded parse must stay bounded across every handoff, so the window travels with
    /// the cursor instead of being re-imposed by each parser adapter. `bounds` keeps the natural
    /// range so a window can be narrowed and restored without losing the owner's extent.
    window_start: usize,
    window_end: usize,
}

#[derive(Clone, Copy, Debug)]
enum TokenCursorBounds<'a> {
    Contiguous(TokenRange),
    Segmented(TokenSequenceView<'a>),
}

impl<'a> TokenCursor<'a> {
    /// Return the canonical source owner behind this short-lived cursor.
    ///
    /// This is used only to create checked nested/range cursors. No caller may retain the cursor
    /// or source reference in declaration syntax shells.
    pub(crate) const fn source_tokens(self) -> &'a SourceTokens {
        self.tokens
    }
    /// Whether this cursor is backed by a segmented logical token sequence.
    pub(crate) const fn is_segmented(self) -> bool {
        matches!(self.bounds, TokenCursorBounds::Segmented(_))
    }

    /// Return the parser-facing position while retaining raw source positions in `position`.
    ///
    /// Contiguous cursors use absolute source indexes inside their active range. Segmented
    /// cursors expose a dense compatibility position that skips omitted source gaps.
    pub(crate) fn parser_position(self) -> usize {
        match self.bounds {
            TokenCursorBounds::Contiguous(_) => self.next.index(),
            TokenCursorBounds::Segmented(_) => self.logical_position,
        }
    }
    /// Map the end of a checked nested range into this cursor's parser position space.
    ///
    /// Contiguous ranges use absolute source indexes. Segmented ranges use dense positions, so
    /// the raw source offset is translated relative to the current segment without scanning
    /// earlier segments.
    pub(crate) fn parser_position_at_range_end(self, range: TokenRange) -> Option<usize> {
        match self.bounds {
            TokenCursorBounds::Contiguous(_) => Some(range.end().index()),
            TokenCursorBounds::Segmented(_) => {
                let current = self.current_range()?;
                if range.source() != current.source()
                    || range.start() < current.start()
                    || range.end() > current.end()
                {
                    return None;
                }
                let current_offset = self.next.index().checked_sub(current.start().index())?;
                let range_offset = range.end().index().checked_sub(current.start().index())?;
                self.logical_position
                    .checked_sub(current_offset)?
                    .checked_add(range_offset)
            }
        }
    }

    /// Return the active parser view's lower bound in parser coordinates.
    pub(crate) const fn parser_window_start(self) -> usize {
        self.window_start
    }

    /// Return the active parser view's exclusive upper bound in parser coordinates.
    ///
    /// Parser adapters read this as the end of what they may parse, so it reports the window
    /// rather than the owner's natural extent.
    pub(crate) const fn parser_length(self) -> usize {
        self.window_end
    }

    /// Whether `position` lies inside the active parser view.
    const fn position_in_window(self, position: usize) -> bool {
        position >= self.window_start && position < self.window_end
    }

    /// Narrow the active parser view to `[start, end)` and return the replaced window.
    ///
    /// Narrowing never widens: a window outside the current one is rejected so a child parse
    /// cannot recover tokens its parent already excluded.
    pub(crate) fn narrow_parser_window(
        &mut self,
        start: usize,
        end: usize,
    ) -> Result<(usize, usize), CompilerError> {
        if start > end {
            return Err(CompilerError::compiler_error(
                "token cursor parser window is inverted",
            ));
        }
        if start < self.window_start || end > self.window_end {
            return Err(CompilerError::compiler_error(
                "token cursor parser window would widen its parent view",
            ));
        }
        let previous = (self.window_start, self.window_end);
        self.window_start = start;
        self.window_end = end;
        Ok(previous)
    }

    /// Restore a window returned by `narrow_parser_window`.
    pub(crate) const fn restore_parser_window(&mut self, window: (usize, usize)) {
        (self.window_start, self.window_end) = window;
    }

    pub fn new(tokens: &'a SourceTokens, range: TokenRange) -> Result<Self, TokenRangeError> {
        tokens.validate_range(range)?;
        Ok(Self {
            tokens,
            bounds: TokenCursorBounds::Contiguous(range),
            segment_index: 0,
            next: range.start,
            logical_position: 0,
            logical_length: 0,
            segment_start_position: 0,
            previous_segment_index: None,
            next_segment_index: None,
            window_start: range.start.index(),
            window_end: range.end.index(),
        })
    }

    #[cfg(test)]
    pub fn from_bounds(
        tokens: &'a SourceTokens,
        start: TokenIndex,
        end: TokenIndex,
    ) -> Result<Self, TokenRangeError> {
        Self::new(tokens, TokenRange::try_new_for(tokens, start, end)?)
    }

    /// Construct a segmented cursor at a checked compatibility-stream position.
    ///
    /// A sequence's compatibility position counts materialised tokens, not source indexes; this
    /// preserves omitted source gaps while still allowing a parser handoff at any token boundary.
    fn from_sequence_position(
        view: TokenSequenceView<'a>,
        compatibility_position: usize,
        logical_length: usize,
    ) -> Result<Self, TokenSequenceError> {
        let ranges = view.tokens.sequence_store.ranges(view.id)?;
        if compatibility_position > logical_length {
            return Err(TokenSequenceError::OutOfRange {
                raw: u32::try_from(compatibility_position).unwrap_or(u32::MAX),
                len: logical_length,
            });
        }

        let mut logical_start = 0usize;
        let mut previous_segment_index = None;
        for (segment_index, entry) in ranges.iter().enumerate() {
            let segment_len = entry.len() as usize;
            let segment_end = logical_start
                .checked_add(segment_len)
                .ok_or(TokenSequenceError::Capacity)?;
            if compatibility_position < segment_end {
                let offset = compatibility_position
                    .checked_sub(logical_start)
                    .ok_or(TokenSequenceError::Capacity)?;
                let offset = u32::try_from(offset).map_err(|_| TokenSequenceError::Capacity)?;
                let next = TokenIndex(
                    entry
                        .start()
                        .raw()
                        .checked_add(offset)
                        .ok_or(TokenSequenceError::Capacity)?,
                );
                return Ok(Self {
                    tokens: view.tokens,
                    bounds: TokenCursorBounds::Segmented(view),
                    segment_index,
                    next,
                    logical_position: compatibility_position,
                    logical_length,
                    segment_start_position: logical_start,
                    previous_segment_index,
                    next_segment_index: Self::next_non_empty_segment(ranges, segment_index + 1),
                    window_start: 0,
                    window_end: logical_length,
                });
            }
            logical_start = segment_end;
            if entry.start() != entry.end() {
                previous_segment_index = Some(segment_index);
            }
        }

        debug_assert_eq!(logical_start, logical_length);
        let next = ranges
            .last()
            .map(|entry| entry.end())
            .unwrap_or(TokenIndex(0));
        Ok(Self {
            tokens: view.tokens,
            bounds: TokenCursorBounds::Segmented(view),
            segment_index: ranges.len(),
            next,
            logical_position: logical_length,
            logical_length,
            segment_start_position: logical_length,
            previous_segment_index,
            next_segment_index: None,
            window_start: 0,
            window_end: logical_length,
        })
    }

    pub fn from_sequence(view: TokenSequenceView<'a>) -> Result<Self, TokenSequenceError> {
        let logical_length = view.len();
        Self::from_sequence_position(view, 0, logical_length)
    }

    pub fn range(self) -> TokenRange {
        match self.bounds {
            TokenCursorBounds::Contiguous(range) => range,
            TokenCursorBounds::Segmented(view) => self.current_range().unwrap_or_else(|| {
                TokenRange::new(view.source(), self.next, self.next)
                    .expect("zero-width token range is ordered")
            }),
        }
    }

    pub const fn position(self) -> TokenIndex {
        self.next
    }

    fn next_non_empty_segment(ranges: &[TokenSequenceRange], start: usize) -> Option<usize> {
        ranges
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, range)| (range.start() != range.end()).then_some(index))
    }
    fn previous_non_empty_segment(ranges: &[TokenSequenceRange], before: usize) -> Option<usize> {
        let mut index = before;
        while index > 0 {
            index -= 1;
            if ranges
                .get(index)
                .is_some_and(|range| range.start() != range.end())
            {
                return Some(index);
            }
        }
        None
    }

    /// Read a token in the bounded parser-facing view.
    ///
    /// Contiguous reads accept absolute source indexes inside the active range. Segmented
    /// reads accept dense sequence positions that skip omitted source gaps.
    pub(crate) fn parser_token_at(self, index: usize) -> Option<TokenRef<'a>> {
        if !self.position_in_window(index) {
            return None;
        }
        match self.bounds {
            TokenCursorBounds::Contiguous(_) => {
                let index = TokenIndex::try_from_index(index)?;
                self.tokens.token(index).ok()
            }
            TokenCursorBounds::Segmented(view) => {
                if index == self.logical_position {
                    return self.current();
                }
                if index == self.logical_position.checked_add(1)? {
                    return self.parser_peek_next();
                }
                if index.checked_add(1) == Some(self.logical_position) {
                    return self.parser_previous();
                }
                Self::from_sequence_position(view, index, self.logical_length)
                    .ok()?
                    .current()
            }
        }
    }

    /// Peek one token ahead in the parser-facing view.
    pub(crate) fn parser_peek_next(self) -> Option<TokenRef<'a>> {
        if !self.position_in_window(self.parser_position().checked_add(1)?) {
            return None;
        }
        let TokenCursorBounds::Segmented(view) = self.bounds else {
            return self.peek_next();
        };
        let range = self.current_range()?;
        if let Some(next) = self.next.raw().checked_add(1)
            && next < range.end().raw()
        {
            return self.tokens.token(TokenIndex(next)).ok();
        }
        let segment_index = self.next_segment_index?;
        let next_range = view.range_at(segment_index).ok()?;
        self.tokens.token(next_range.start()).ok()
    }

    /// Return the previous token in the parser-facing view.
    pub(crate) fn parser_previous(self) -> Option<TokenRef<'a>> {
        if !self.position_in_window(self.parser_position().checked_sub(1)?) {
            return None;
        }
        let TokenCursorBounds::Segmented(view) = self.bounds else {
            let previous = TokenIndex::try_from_raw(self.next.raw().checked_sub(1)?)?;
            if previous < self.range().start() {
                return None;
            }
            return self.tokens.token(previous).ok();
        };
        if self.logical_position == 0 {
            return None;
        }
        if let Some(range) = self.current_range()
            && self.next > range.start()
        {
            let previous = TokenIndex::try_from_raw(self.next.raw().checked_sub(1)?)?;
            return self.tokens.token(previous).ok();
        }
        let previous_index = self.previous_segment_index?;
        let previous_range = view.range_at(previous_index).ok()?;
        let previous = TokenIndex::try_from_raw(previous_range.end().raw().checked_sub(1)?)?;
        self.tokens.token(previous).ok()
    }
    /// Move a segmented cursor to a dense parser-facing position.
    pub(crate) fn set_parser_position(
        &mut self,
        position: usize,
    ) -> Result<(), TokenSequenceError> {
        let TokenCursorBounds::Segmented(view) = self.bounds else {
            return Err(TokenSequenceError::Absent);
        };
        if position < self.window_start || position > self.window_end {
            return Err(TokenSequenceError::OutOfRange {
                raw: u32::try_from(position).unwrap_or(u32::MAX),
                len: self.window_end,
            });
        }
        if position == self.logical_position {
            return Ok(());
        }
        let ranges = view
            .tokens
            .sequence_store
            .ranges(view.id)
            .expect("validated token sequence view has a valid handle");

        if position > self.logical_position {
            loop {
                let Some(range) = ranges.get(self.segment_index).copied() else {
                    return Err(TokenSequenceError::OutOfRange {
                        raw: u32::try_from(position).unwrap_or(u32::MAX),
                        len: self.logical_length,
                    });
                };
                let segment_end = self
                    .segment_start_position
                    .checked_add(range.len() as usize)
                    .ok_or(TokenSequenceError::Capacity)?;
                if position <= segment_end {
                    let offset = position
                        .checked_sub(self.segment_start_position)
                        .ok_or(TokenSequenceError::Capacity)?;
                    let offset = u32::try_from(offset).map_err(|_| TokenSequenceError::Capacity)?;
                    self.next = TokenIndex(
                        range
                            .start()
                            .raw()
                            .checked_add(offset)
                            .ok_or(TokenSequenceError::Capacity)?,
                    );
                    self.logical_position = position;
                    if position == segment_end {
                        self.skip_empty_segments();
                    }
                    return Ok(());
                }
                let next_segment_index =
                    self.next_segment_index
                        .ok_or(TokenSequenceError::OutOfRange {
                            raw: u32::try_from(position).unwrap_or(u32::MAX),
                            len: self.logical_length,
                        })?;
                self.previous_segment_index = Some(self.segment_index);
                self.segment_index = next_segment_index;
                self.segment_start_position = segment_end;
                self.next = ranges[next_segment_index].start();
                self.next_segment_index =
                    Self::next_non_empty_segment(ranges, next_segment_index + 1);
            }
        }

        loop {
            if let Some(range) = ranges.get(self.segment_index).copied()
                && position >= self.segment_start_position
            {
                let offset = position
                    .checked_sub(self.segment_start_position)
                    .ok_or(TokenSequenceError::Capacity)?;
                let offset = u32::try_from(offset).map_err(|_| TokenSequenceError::Capacity)?;
                self.next = TokenIndex(
                    range
                        .start()
                        .raw()
                        .checked_add(offset)
                        .ok_or(TokenSequenceError::Capacity)?,
                );
                self.logical_position = position;
                return Ok(());
            }
            let previous_segment_index =
                self.previous_segment_index
                    .ok_or(TokenSequenceError::OutOfRange {
                        raw: u32::try_from(position).unwrap_or(u32::MAX),
                        len: self.logical_length,
                    })?;
            let previous_range = ranges[previous_segment_index];
            self.segment_index = previous_segment_index;
            self.segment_start_position = self
                .segment_start_position
                .checked_sub(previous_range.len() as usize)
                .ok_or(TokenSequenceError::Capacity)?;
            self.next = previous_range.start();
            self.next_segment_index =
                Self::next_non_empty_segment(ranges, previous_segment_index + 1);
            self.previous_segment_index =
                Self::previous_non_empty_segment(ranges, previous_segment_index);
        }
    }

    /// Whether the next token starts a later segmented range with omitted source bytes.
    ///
    /// Contiguous cursors always return `false`. Adjacent ranges are still one logical token
    /// stream; only a real source gap must prevent consumers from comparing the two tokens as an
    /// authored pair.
    pub(crate) fn is_at_segment_start(self) -> bool {
        let TokenCursorBounds::Segmented(view) = self.bounds else {
            return false;
        };
        let Some(current) = view.range_at(self.segment_index).ok() else {
            return false;
        };
        if self.next != current.start() {
            return false;
        }
        let Some(previous_index) = self.previous_segment_index else {
            return false;
        };
        view.range_at(previous_index)
            .ok()
            .is_some_and(|previous| previous.end() != current.start())
    }

    fn current_range(self) -> Option<TokenRange> {
        match self.bounds {
            TokenCursorBounds::Contiguous(range) => Some(range),
            TokenCursorBounds::Segmented(view) => view.range_at(self.segment_index).ok(),
        }
    }

    fn skip_empty_segments(&mut self) {
        let TokenCursorBounds::Segmented(view) = self.bounds else {
            return;
        };
        let ranges = view
            .tokens
            .sequence_store
            .ranges(view.id)
            .expect("validated token sequence view has a valid handle");
        loop {
            let Some(range) = ranges.get(self.segment_index).copied() else {
                self.segment_start_position = self.logical_length;
                self.next_segment_index = None;
                return;
            };
            if self.next < range.start() {
                self.next = range.start();
            }
            if self.next < range.end() {
                self.next_segment_index =
                    Self::next_non_empty_segment(ranges, self.segment_index + 1);
                return;
            }
            if range.start() != range.end() {
                self.previous_segment_index = Some(self.segment_index);
            }
            self.segment_index = self.segment_index.saturating_add(1);
            self.segment_start_position = self
                .segment_start_position
                .saturating_add(range.len() as usize);
            self.next = ranges
                .get(self.segment_index)
                .map(|next_range| next_range.start())
                .unwrap_or(range.end());
        }
    }

    pub fn is_at_end(self) -> bool {
        self.current().is_none()
    }

    pub fn current(self) -> Option<TokenRef<'a>> {
        if !self.position_in_window(self.parser_position()) {
            return None;
        }
        let range = self.current_range()?;
        if self.next >= range.end {
            return None;
        }
        self.tokens.token(self.next).ok()
    }

    /// Peek at the current token without consuming it.
    pub fn peek(self) -> Option<TokenRef<'a>> {
        self.current()
    }

    /// Peek one token after the current cursor position.
    ///
    /// A segmented cursor never crosses a range boundary for `peek_next`.
    pub fn peek_next(self) -> Option<TokenRef<'a>> {
        if !self.position_in_window(self.parser_position().checked_add(1)?) {
            return None;
        }
        let range = self.current_range()?;
        let next = TokenIndex(self.next.raw().checked_add(1)?);
        if next >= range.end {
            None
        } else {
            self.tokens.token(next).ok()
        }
    }

    /// Move to a checked position in the active token view.
    ///
    /// Segmented cursors accept positions inside one segment, including that segment's end
    /// sentinel, but never fabricate a position in an omitted source gap.
    pub fn set_position(&mut self, position: TokenIndex) -> Result<(), TokenRangeError> {
        match self.bounds {
            TokenCursorBounds::Contiguous(range) => {
                if position.index() < self.window_start || position.index() > self.window_end {
                    return Err(TokenRangeError::OutOfBounds {
                        start: position.raw(),
                        end: position.raw(),
                        len: self.window_end,
                    });
                }
                if position < range.start || position > range.end {
                    return Err(TokenRangeError::OutOfBounds {
                        start: position.raw(),
                        end: position.raw(),
                        len: range.end.index(),
                    });
                }
                self.next = position;
                Ok(())
            }
            TokenCursorBounds::Segmented(view) => {
                let ranges = view
                    .tokens
                    .sequence_store
                    .ranges(view.id)
                    .expect("validated token sequence view has a valid handle");
                let logical_length = self.logical_length;
                let mut logical_start = 0usize;
                let mut previous_segment_index = None;
                for (segment_index, entry) in ranges.iter().copied().enumerate() {
                    if position >= entry.start() && position <= entry.end() {
                        let offset = (position.raw() - entry.start().raw()) as usize;
                        let mut candidate = Self {
                            tokens: view.tokens,
                            bounds: TokenCursorBounds::Segmented(view),
                            segment_index,
                            next: position,
                            logical_position: logical_start.saturating_add(offset),
                            logical_length,
                            segment_start_position: logical_start,
                            previous_segment_index,
                            next_segment_index: Self::next_non_empty_segment(
                                ranges,
                                segment_index + 1,
                            ),
                            window_start: self.window_start,
                            window_end: self.window_end,
                        };
                        if candidate.logical_position < self.window_start
                            || candidate.logical_position > self.window_end
                        {
                            return Err(TokenRangeError::OutOfBounds {
                                start: position.raw(),
                                end: position.raw(),
                                len: self.window_end,
                            });
                        }
                        candidate.skip_empty_segments();
                        *self = candidate;
                        return Ok(());
                    }
                    logical_start = logical_start.saturating_add(entry.len() as usize);
                    if entry.start() != entry.end() {
                        previous_segment_index = Some(segment_index);
                    }
                }
                Err(TokenRangeError::OutOfBounds {
                    start: position.raw(),
                    end: position.raw(),
                    len: logical_length,
                })
            }
        }
    }

    /// Whether the current token view is at EOF.
    #[cfg(test)]
    pub fn is_eof(self) -> bool {
        self.current().is_none_or(TokenRef::is_eof)
    }

    /// Return the current token and move to the next one. EOF is stable and never advances.
    pub fn advance(&mut self) -> Option<TokenRef<'a>> {
        let current = self.current()?;
        if current.is_eof() {
            return Some(current);
        }

        let range = self
            .current_range()
            .expect("a current token always belongs to a cursor range");
        if matches!(self.bounds, TokenCursorBounds::Segmented(_)) {
            self.logical_position = self.logical_position.saturating_add(1);
        }
        if let Some(next) = self.next.raw().checked_add(1)
            && next < range.end.raw()
        {
            self.next = TokenIndex(next);
        } else {
            self.next = range.end;
            if matches!(self.bounds, TokenCursorBounds::Segmented(_)) {
                self.previous_segment_index = Some(self.segment_index);
                self.segment_index = self.segment_index.saturating_add(1);
                self.segment_start_position = self
                    .segment_start_position
                    .saturating_add(range.len() as usize);
                self.skip_empty_segments();
            }
        }
        Some(current)
    }

    /// Create a nested cursor after proving that the child range is inside the active view.
    ///
    /// The child inherits the parent's window intersected with its own range, so a nested parse
    /// can never recover tokens the parent excluded. A segmented parent's window is expressed in
    /// dense positions, which do not translate into the contiguous child's coordinates, so the
    /// child takes its own range as its window; callers translate dense containment first.
    pub fn nested(&self, range: TokenRange) -> Result<TokenCursor<'a>, TokenRangeError> {
        self.tokens.validate_range(range)?;
        let parent = self.current_range().unwrap_or_else(|| self.range());
        if range.start < parent.start || range.end > parent.end {
            return Err(TokenRangeError::OutOfBounds {
                start: range.start.raw(),
                end: range.end.raw(),
                len: parent.end.index(),
            });
        }
        let mut nested = Self::new(self.tokens, range)?;
        if matches!(self.bounds, TokenCursorBounds::Contiguous(_)) {
            if range.start.index() < self.window_start || range.end.index() > self.window_end {
                return Err(TokenRangeError::OutOfBounds {
                    start: range.start.raw(),
                    end: range.end.raw(),
                    len: self.window_end,
                });
            }
            nested.window_start = range.start.index().max(self.window_start);
            nested.window_end = range.end.index().min(self.window_end);
        }
        Ok(nested)
    }
}
