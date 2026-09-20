//! Canonical source-token storage and bounded sequence ownership.

use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::numeric_text::store::{
    NumericLiteralId, NumericLiteralStore, NumericLiteralStoreError,
};
#[cfg(test)]
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
#[cfg(test)]
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxError, PathSyntaxTable};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SpanCapacityError};
#[cfg(test)]
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringIdRemap};
use std::sync::Arc;

use super::cursor::{TokenCursor, TokenRef};
use super::schema::{TokenShape, TokenTag};
/// A checked zero-based index into one source's token arrays.
///
/// The packed representation is deliberately `u32`; conversion to `usize` happens only at the
/// indexing boundary and is never retained in source-owned token data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenIndex(pub(super) u32);

impl TokenIndex {
    /// Construct an index from its packed representation.
    ///
    /// Every `u32` value is representable. Whether it addresses a token is checked by
    /// [`SourceTokens::token`] or [`TokenCursor::new`].
    pub const fn try_from_raw(raw: u32) -> Option<Self> {
        Some(Self(raw))
    }

    /// Construct an index from a host index without truncation.
    pub const fn try_from_index(index: usize) -> Option<Self> {
        if index > u32::MAX as usize {
            None
        } else {
            Some(Self(index as u32))
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}
/// Check the half-open length domain of one source token store.
///
/// A token index may address `u32::MAX`, because that value is a valid zero-based index.
/// The store length is different: its half-open endpoint must fit in the same `u32`
/// representation, so `u32::MAX` is the largest valid length and the next length is rejected.
pub(crate) const fn token_store_length_fits(length: usize) -> bool {
    length <= u32::MAX as usize
}
/// Check whether appending one token keeps the half-open store length in its `u32` domain.
pub(crate) const fn token_store_append_fits(current_length: usize) -> bool {
    match current_length.checked_add(1) {
        Some(next_length) => token_store_length_fits(next_length),
        None => false,
    }
}

/// A half-open token interval `[start, end)` qualified by its source identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenRange {
    pub(super) source: SourceId,
    pub(super) start: TokenIndex,
    pub(super) end: TokenIndex,
}

impl TokenRange {
    pub const fn new(source: SourceId, start: TokenIndex, end: TokenIndex) -> Option<Self> {
        if start.raw() <= end.raw() {
            Some(Self { source, start, end })
        } else {
            None
        }
    }

    /// Construct an ordered range from packed raw indexes.
    pub const fn from_raw(source: SourceId, start: u32, end: u32) -> Option<Self> {
        Self::new(source, TokenIndex(start), TokenIndex(end))
    }

    /// Construct a range and validate both its source and bounds against `tokens`.
    pub fn try_new_for(
        tokens: &SourceTokens,
        start: TokenIndex,
        end: TokenIndex,
    ) -> Result<Self, TokenRangeError> {
        let range = Self::new(tokens.source(), start, end).ok_or(TokenRangeError::Reversed {
            start: start.raw(),
            end: end.raw(),
        })?;
        tokens.validate_range(range)?;
        Ok(range)
    }

    pub const fn source(self) -> SourceId {
        self.source
    }

    pub const fn start(self) -> TokenIndex {
        self.start
    }

    pub const fn end(self) -> TokenIndex {
        self.end
    }

    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }

    pub const fn len(self) -> u32 {
        self.end.0 - self.start.0
    }

    /// Restamp this range with the final build-lifetime source identity.
    ///
    /// Token indexes are source-local and therefore remain unchanged across the preparation
    /// rebinding boundary. Callers must validate the returned range against the final owner
    /// before publishing it.
    pub const fn rebind_source(self, source: SourceId) -> Self {
        Self {
            source,
            start: self.start,
            end: self.end,
        }
    }
}

/// Failures at the checked token range/index boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenRangeError {
    ForeignSource {
        expected: SourceId,
        actual: SourceId,
    },
    Reversed {
        start: u32,
        end: u32,
    },
    OutOfBounds {
        start: u32,
        end: u32,
        len: usize,
    },
}
/// A checked source-local range entry in a token sequence.
///
/// The entry intentionally stores only the two zero-based token indexes. The owning
/// [`TokenSequenceStore`] carries the [`SourceId`] once for every sequence in that source.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenSequenceRange {
    start: TokenIndex,
    end: TokenIndex,
}

const _: () = assert!(std::mem::size_of::<TokenSequenceRange>() == 8);

impl TokenSequenceRange {
    pub const fn start(self) -> TokenIndex {
        self.start
    }

    pub const fn end(self) -> TokenIndex {
        self.end
    }

    pub const fn len(self) -> u32 {
        self.end.raw() - self.start.raw()
    }
}

/// A checked one-based handle into one source's segmented token sequence store.
///
/// Zero is reserved as the absent marker and is never a valid sequence identity.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenSequenceId(u32);

impl TokenSequenceId {
    #[cfg(test)]
    pub const NONE: Self = Self(0);

    #[cfg(test)]
    pub const fn try_from_raw(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn try_from_index(index: usize) -> Option<Self> {
        if index >= u32::MAX as usize {
            return None;
        }
        Some(Self((index as u32) + 1))
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn index(self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some((self.0 - 1) as usize)
        }
    }
}

/// Failures while constructing or resolving a source-local token sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenSequenceError {
    ForeignSource {
        expected: SourceId,
        actual: SourceId,
    },
    Reversed {
        start: u32,
        end: u32,
    },
    OutOfBounds {
        start: u32,
        end: u32,
        len: usize,
    },
    Unordered {
        previous_start: u32,
        start: u32,
    },
    Overlapping {
        previous_end: u32,
        start: u32,
    },
    Absent,
    OutOfRange {
        raw: u32,
        len: usize,
    },
    NoCanonicalOwner,
    Capacity,
    Frozen,
}

/// One source-owned flat range-list store for segmented token sequences.
///
/// `ranges` is the only per-segment payload: every row is exactly `{ start, end }`.
/// `sequence_offsets` is a compact index into that flat list; its trailing sentinel
/// makes each one-based [`TokenSequenceId`] resolve without storing source/path data.
#[cfg_attr(test, derive(Clone))]
#[derive(Debug)]
pub struct TokenSequenceStore {
    source: SourceId,
    token_len: usize,
    ranges: Vec<TokenSequenceRange>,
    sequence_offsets: Vec<u32>,
    frozen: bool,
}

impl TokenSequenceStore {
    pub fn new(source: SourceId, token_len: usize) -> Self {
        Self {
            source,
            token_len,
            ranges: Vec::new(),
            sequence_offsets: vec![0],
            frozen: false,
        }
    }

    #[cfg(test)]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    pub fn len(&self) -> usize {
        self.sequence_offsets.len().saturating_sub(1)
    }

    pub fn ranges(&self, id: TokenSequenceId) -> Result<&[TokenSequenceRange], TokenSequenceError> {
        let index = id.index().ok_or(TokenSequenceError::Absent)?;
        let Some(&start) = self.sequence_offsets.get(index) else {
            return Err(TokenSequenceError::OutOfRange {
                raw: id.raw(),
                len: self.len(),
            });
        };
        let Some(end_index) = index.checked_add(1) else {
            return Err(TokenSequenceError::OutOfRange {
                raw: id.raw(),
                len: self.len(),
            });
        };
        let Some(&end) = self.sequence_offsets.get(end_index) else {
            return Err(TokenSequenceError::OutOfRange {
                raw: id.raw(),
                len: self.len(),
            });
        };
        let start = start as usize;
        let end = end as usize;
        if start > end || end > self.ranges.len() {
            return Err(TokenSequenceError::OutOfRange {
                raw: id.raw(),
                len: self.len(),
            });
        }
        self.ranges
            .get(start..end)
            .ok_or(TokenSequenceError::OutOfRange {
                raw: id.raw(),
                len: self.len(),
            })
    }

    pub fn try_push(
        &mut self,
        ranges: &[TokenRange],
    ) -> Result<TokenSequenceId, TokenSequenceError> {
        if self.frozen {
            return Err(TokenSequenceError::Frozen);
        }

        let mut previous: Option<TokenRange> = None;
        for range in ranges {
            self.validate_range(*range)?;
            if let Some(previous_range) = previous {
                if range.start() < previous_range.start() {
                    return Err(TokenSequenceError::Unordered {
                        previous_start: previous_range.start().raw(),
                        start: range.start().raw(),
                    });
                }
                if range.start() < previous_range.end() {
                    return Err(TokenSequenceError::Overlapping {
                        previous_end: previous_range.end().raw(),
                        start: range.start().raw(),
                    });
                }
            }
            previous = Some(*range);
        }

        let sequence_index = self.len();
        let id =
            TokenSequenceId::try_from_index(sequence_index).ok_or(TokenSequenceError::Capacity)?;
        let end = self
            .ranges
            .len()
            .checked_add(ranges.len())
            .filter(|len| *len <= u32::MAX as usize)
            .ok_or(TokenSequenceError::Capacity)?;
        self.ranges
            .extend(ranges.iter().map(|range| TokenSequenceRange {
                start: range.start(),
                end: range.end(),
            }));
        self.sequence_offsets
            .push(u32::try_from(end).map_err(|_| TokenSequenceError::Capacity)?);
        Ok(id)
    }

    pub fn validate_id(&self, id: TokenSequenceId) -> Result<(), TokenSequenceError> {
        self.ranges(id).map(|_| ())
    }

    pub fn rebind_source_identity(&mut self, source: SourceId) {
        self.source = source;
    }

    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    fn validate_range(&self, range: TokenRange) -> Result<(), TokenSequenceError> {
        if range.source() != self.source {
            return Err(TokenSequenceError::ForeignSource {
                expected: self.source,
                actual: range.source(),
            });
        }
        if range.start() > range.end() {
            return Err(TokenSequenceError::Reversed {
                start: range.start().raw(),
                end: range.end().raw(),
            });
        }
        if range.end().index() > self.token_len {
            return Err(TokenSequenceError::OutOfBounds {
                start: range.start().raw(),
                end: range.end().raw(),
                len: self.token_len,
            });
        }
        Ok(())
    }
}

/// A borrowed view over one source-local segmented token sequence.
#[derive(Clone, Copy, Debug)]
pub struct TokenSequenceView<'a> {
    pub(super) tokens: &'a SourceTokens,
    pub(super) id: TokenSequenceId,
}

impl<'a> TokenSequenceView<'a> {
    fn new(tokens: &'a SourceTokens, id: TokenSequenceId) -> Result<Self, TokenSequenceError> {
        tokens.sequence_store.validate_id(id)?;
        Ok(Self { tokens, id })
    }

    pub const fn source(self) -> SourceId {
        self.tokens.source()
    }

    pub fn ranges(self) -> impl ExactSizeIterator<Item = TokenRange> + 'a {
        self.tokens
            .sequence_store
            .ranges(self.id)
            .expect("validated token sequence view has a valid handle")
            .iter()
            .copied()
            .map(move |entry| {
                TokenRange::new(self.source(), entry.start(), entry.end())
                    .expect("validated token sequence entry is ordered")
            })
    }

    pub fn range_at(self, index: usize) -> Result<TokenRange, TokenSequenceError> {
        let entry = self
            .tokens
            .sequence_store
            .ranges(self.id)?
            .get(index)
            .copied()
            .ok_or(TokenSequenceError::OutOfRange {
                raw: index as u32,
                len: self.tokens.sequence_store.ranges(self.id)?.len(),
            })?;
        Ok(TokenRange::new(self.source(), entry.start(), entry.end())
            .expect("validated token sequence entry is ordered"))
    }

    pub fn len(self) -> usize {
        self.ranges().map(|range| range.len() as usize).sum()
    }

    pub fn cursor(self) -> Result<TokenCursor<'a>, TokenSequenceError> {
        TokenCursor::from_sequence(self)
    }
}

/// Failures while resolving a token's typed payload view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenViewError {
    OutOfBounds { index: u32, len: usize },
    MalformedStringHandle,
    MissingPathTable,
    MalformedNumericHandle,
    MalformedPathHandle,
}

/// Donor identity carried separately from a borrowed source-token view.
///
/// The canonical owner supplies source indexes, ranges, spans and cold-store handles. This
/// optional context supplies the frozen string table that issued payload IDs when a requester
/// parses a retained foreign body. Requester tables are passed to the `*_in` readers at the use
/// site, so no translation state or second token store is retained here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TokenPayloadOrigin<'a> {
    pub(crate) strings: &'a FrozenStringTable,
}

/// Failures while packing one token into the canonical construction owner.
///
/// WHAT: distinguishes user-controlled token-count exhaustion from malformed trusted records.
/// WHY: the lexer maps these lanes without a second classification pass — capacity becomes a
/// typed user diagnostic, invariants stay infrastructure failures.
#[derive(Clone, Debug)]
pub(crate) enum SourceTokenBuildError {
    Capacity,
    Invariant(CompilerError),
}

/// Failures while emitting one canonical source token.
#[derive(Debug)]
pub(crate) enum TokenEmitError {
    Span(SpanCapacityError),
    Build(SourceTokenBuildError),
    Path(PathSyntaxError),
    Numeric(NumericLiteralStoreError),
}

/// Canonical construction owner for one source's token arrays.
///
/// WHAT: packs compact shapes, spans, stats and the positional numeric handle lane in one
/// monotone forward pass while lexing. The numeric side-store rows themselves stay staged in
/// the lexer's `TokenStream`; this builder consumes only their checked handles.
pub(crate) struct SourceTokensBuilder {
    source: SourceId,
    shapes: Vec<TokenShape>,
    spans: Vec<LocalSpan>,
    token_stats: TokenStats,
    staged_numeric: usize,
}

impl SourceTokensBuilder {
    pub(crate) fn with_capacity(source: SourceId, capacity: usize) -> Self {
        Self {
            source,
            shapes: Vec::with_capacity(capacity),
            spans: Vec::with_capacity(capacity),
            token_stats: TokenStats::default(),
            staged_numeric: 0,
        }
    }

    /// Preflight the half-open token-store endpoint before any canonical mutation.
    pub(crate) fn preflight_next_token(&self) -> Result<(), SourceTokenBuildError> {
        if token_store_append_fits(self.shapes.len()) {
            Ok(())
        } else {
            Err(SourceTokenBuildError::Capacity)
        }
    }

    /// Preflight both the token endpoint and the next positional numeric handle.
    pub(crate) fn preflight_numeric(&self) -> Result<NumericLiteralId, SourceTokenBuildError> {
        self.preflight_next_token()?;
        self.next_numeric_id()
    }

    fn next_numeric_id(&self) -> Result<NumericLiteralId, SourceTokenBuildError> {
        NumericLiteralId::try_from_index(self.staged_numeric).ok_or(SourceTokenBuildError::Capacity)
    }

    /// Push a validated raw shape after its caller has completed any side-store preflight.
    pub(crate) fn push_payload(
        &mut self,
        tag: TokenTag,
        flags: u16,
        data: u32,
        span: LocalSpan,
    ) -> Result<(), SourceTokenBuildError> {
        let next_index = self.shapes.len();
        let shape = TokenShape::from_raw_parts(tag.raw(), flags, data).ok_or_else(|| {
            SourceTokenBuildError::Invariant(CompilerError::compiler_error(format!(
                "trusted token at index {next_index} has a malformed compact shape"
            )))
        })?;
        self.push_packed(shape, span)
    }

    /// Push a numeric shape with the handle preflighted before the cold-store row was appended.
    pub(crate) fn push_numeric(
        &mut self,
        tag: TokenTag,
        flags: u16,
        numeric_id: NumericLiteralId,
        span: LocalSpan,
    ) -> Result<(), SourceTokenBuildError> {
        let expected = self.preflight_numeric()?;
        if expected != numeric_id {
            return Err(SourceTokenBuildError::Invariant(
                CompilerError::compiler_error(
                    "numeric literal handle did not match the staged canonical position",
                ),
            ));
        }
        self.push_payload(tag, flags, numeric_id.raw(), span)?;
        self.staged_numeric += 1;
        Ok(())
    }

    fn push_packed(
        &mut self,
        shape: TokenShape,
        span: LocalSpan,
    ) -> Result<(), SourceTokenBuildError> {
        // Keep this check immediately before every array/statistics mutation. The caller-facing
        // preflight methods additionally protect side-store mutation in typed emitters.
        self.preflight_next_token()?;
        self.token_stats.accumulate_shape(shape);
        self.shapes.push(shape);
        self.spans.push(span);
        Ok(())
    }

    pub(crate) fn finish(
        self,
        numeric_literals: NumericLiteralStore,
    ) -> Result<SourceTokens, CompilerError> {
        if numeric_literals.owner_source().is_none() && !numeric_literals.is_empty() {
            return Err(CompilerError::compiler_error(
                "source token numeric store has records but no source identity",
            ));
        }
        if let Some(owner) = numeric_literals.owner_source()
            && owner != self.source
        {
            return Err(CompilerError::compiler_error(
                "source token numeric store does not match its source identity",
            ));
        }
        if self.staged_numeric != numeric_literals.len() {
            return Err(CompilerError::compiler_error(format!(
                "source token numeric handles (staged {}) do not match numeric store rows ({})",
                self.staged_numeric,
                numeric_literals.len()
            )));
        }
        let len = self.shapes.len();
        let owner = SourceTokens {
            source: self.source,
            shapes: self.shapes.into_boxed_slice(),
            spans: self.spans.into_boxed_slice(),
            numeric_literals,
            path_syntax: None,
            sequence_store: TokenSequenceStore::new(self.source, len),
            token_stats: self.token_stats,
        };
        owner.validate_structure()?;
        Ok(owner)
    }
}

/// Canonical source-token fixture builder for tests that need hand-authored syntax.
///
/// This helper writes the same compact shape/span arrays and source-owned cold stores as the
/// lexer. It deliberately exposes typed payload methods instead of a compatibility token enum.
#[cfg(test)]
pub(crate) struct TestSourceTokensBuilder {
    source: SourceId,
    builder: SourceTokensBuilder,
    numeric_literals: NumericLiteralStore,
    path_syntax: PathSyntaxTable,
}

#[cfg(test)]
impl TestSourceTokensBuilder {
    pub(crate) fn new(source: SourceId) -> Self {
        Self {
            source,
            builder: SourceTokensBuilder::with_capacity(source, 8),
            numeric_literals: NumericLiteralStore::with_source(source),
            path_syntax: PathSyntaxTable::with_source(source),
        }
    }

    pub(crate) fn with_path_syntax(source: SourceId, path_syntax: PathSyntaxTable) -> Self {
        Self {
            source,
            builder: SourceTokensBuilder::with_capacity(source, 8),
            numeric_literals: NumericLiteralStore::with_source(source),
            path_syntax,
        }
    }

    pub(crate) fn push_static(
        &mut self,
        tag: TokenTag,
        span: LocalSpan,
    ) -> Result<(), CompilerError> {
        self.builder
            .push_payload(tag, 0, 0, span)
            .map_err(test_shape_error)
    }

    pub(crate) fn push_symbol(
        &mut self,
        tag: TokenTag,
        value: StringId,
        span: LocalSpan,
    ) -> Result<(), CompilerError> {
        self.builder
            .push_payload(tag, 0, value.index(), span)
            .map_err(test_shape_error)
    }

    pub(crate) fn push_path(
        &mut self,
        tag: TokenTag,
        value: PathSyntaxId,
        span: LocalSpan,
    ) -> Result<(), CompilerError> {
        self.builder
            .push_payload(tag, 0, value.raw(), span)
            .map_err(test_shape_error)
    }

    pub(crate) fn push_char(
        &mut self,
        tag: TokenTag,
        value: char,
        span: LocalSpan,
    ) -> Result<(), CompilerError> {
        self.builder
            .push_payload(tag, 0, value as u32, span)
            .map_err(test_shape_error)
    }

    pub(crate) fn push_bool(
        &mut self,
        tag: TokenTag,
        value: bool,
        span: LocalSpan,
    ) -> Result<(), CompilerError> {
        self.builder
            .push_payload(tag, 0, u32::from(value), span)
            .map_err(test_shape_error)
    }

    pub(crate) fn push_numeric(
        &mut self,
        literal: NumericLiteralToken,
        span: LocalSpan,
    ) -> Result<(), CompilerError> {
        let numeric_id = self.builder.preflight_numeric().map_err(test_shape_error)?;
        let kind = literal.kind;
        self.numeric_literals.try_push(literal).map_err(|error| {
            CompilerError::compiler_error(format!(
                "test numeric literal insertion failed: {error:?}"
            ))
        })?;
        self.builder
            .push_numeric(
                TokenTag::NUMERIC_LITERAL,
                super::schema::numeric_kind_flags(kind),
                numeric_id,
                span,
            )
            .map_err(test_shape_error)
    }

    pub(crate) fn finish(self) -> Result<Arc<SourceTokens>, CompilerError> {
        let mut source_tokens = self.finish_unfrozen()?;
        Arc::get_mut(&mut source_tokens)
            .expect("test source token owner should be unique before freeze")
            .freeze_numeric_literals();
        Ok(source_tokens)
    }

    /// Finish the canonical fixture owner while leaving its sequence store mutable.
    ///
    /// Segmented fixtures register their checked ranges before publishing the final frozen
    /// owner. Callers must freeze numeric and sequence storage after registration.
    pub(crate) fn finish_unfrozen(self) -> Result<Arc<SourceTokens>, CompilerError> {
        self.path_syntax
            .validate_file_owned_locations(self.source)?;
        let mut source_tokens = self.builder.finish(self.numeric_literals)?;
        let mut path_syntax = self.path_syntax;
        path_syntax.freeze();
        source_tokens.attach_shared_path_syntax(Arc::new(path_syntax));
        Ok(Arc::new(source_tokens))
    }
}

#[cfg(test)]
fn test_shape_error(error: SourceTokenBuildError) -> CompilerError {
    match error {
        SourceTokenBuildError::Capacity => {
            CompilerError::compiler_error("test source token fixture exceeded its capacity")
        }
        SourceTokenBuildError::Invariant(error) => error,
    }
}

/// The immutable source-owned shape/span arrays and their typed cold stores.
///
/// The canonical owner stores frozen dense shape/span arrays and source-local numeric, path,
/// and segmented-sequence rows. Cursors and typed references read this owner without
/// materialising a second token representation.
#[cfg_attr(test, derive(Clone))]
#[derive(Debug)]
pub struct SourceTokens {
    pub(super) source: SourceId,
    pub(super) shapes: Box<[TokenShape]>,
    pub(super) spans: Box<[LocalSpan]>,
    pub(super) numeric_literals: NumericLiteralStore,
    /// A path table is attached once its preparation owner reaches the immutable boundary.
    ///
    /// Direct source-store construction installs the shared immutable table without copying rows.
    pub(super) path_syntax: Option<Arc<PathSyntaxTable>>,
    pub(super) sequence_store: TokenSequenceStore,
    pub(super) token_stats: TokenStats,
}

impl SourceTokens {
    /// Return this store's source identity.
    pub const fn source(&self) -> SourceId {
        self.source
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn shapes(&self) -> &[TokenShape] {
        &self.shapes
    }
    #[cfg(test)]
    pub(crate) fn corrupt_payload_for_test(&mut self, index: usize, tag: TokenTag) {
        self.shapes[index] =
            TokenShape::new(tag, 0, 0).expect("the test corruption tag must accept zero flags");
    }

    pub fn spans(&self) -> &[LocalSpan] {
        &self.spans
    }

    #[cfg(test)]
    pub(crate) fn numeric_literal_store(&self) -> &NumericLiteralStore {
        &self.numeric_literals
    }

    pub(crate) fn token_stats(&self) -> TokenStats {
        self.token_stats
    }

    pub(crate) fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        self.path_syntax.as_deref().ok_or_else(|| {
            CompilerError::compiler_error(
                "source token path table is not attached at its immutable publication boundary",
            )
        })
    }

    pub(crate) fn path_syntax_arc(&self) -> Result<Arc<PathSyntaxTable>, CompilerError> {
        self.path_syntax.clone().ok_or_else(|| {
            CompilerError::compiler_error(
                "source token path table is not attached at its immutable publication boundary",
            )
        })
    }

    fn validate_structure(&self) -> Result<(), CompilerError> {
        if self.shapes.len() != self.spans.len() {
            return Err(CompilerError::compiler_error(
                "source token shape/span arrays have different lengths",
            ));
        }
        if !token_store_length_fits(self.shapes.len()) {
            return Err(CompilerError::compiler_error(
                "source token arrays exceed their checked u32 index domain",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_range(&self, range: TokenRange) -> Result<(), TokenRangeError> {
        if range.source != self.source {
            return Err(TokenRangeError::ForeignSource {
                expected: self.source,
                actual: range.source,
            });
        }
        if range.start > range.end {
            return Err(TokenRangeError::Reversed {
                start: range.start.raw(),
                end: range.end.raw(),
            });
        }
        if range.end.index() > self.len() {
            return Err(TokenRangeError::OutOfBounds {
                start: range.start.raw(),
                end: range.end.raw(),
                len: self.len(),
            });
        }
        Ok(())
    }

    pub fn full_range(&self) -> Result<TokenRange, TokenRangeError> {
        let end = TokenIndex::try_from_index(self.len()).ok_or(TokenRangeError::OutOfBounds {
            start: 0,
            end: u32::MAX,
            len: self.len(),
        })?;
        Ok(TokenRange {
            source: self.source,
            start: TokenIndex(0),
            end,
        })
    }

    pub fn range(&self, start: TokenIndex, end: TokenIndex) -> Result<TokenRange, TokenRangeError> {
        TokenRange::try_new_for(self, start, end)
    }

    pub fn token(&self, index: TokenIndex) -> Result<TokenRef<'_>, TokenViewError> {
        if index.index() >= self.len() {
            return Err(TokenViewError::OutOfBounds {
                index: index.raw(),
                len: self.len(),
            });
        }
        Ok(TokenRef {
            tokens: self,
            index,
        })
    }

    pub fn cursor(&self, range: TokenRange) -> Result<TokenCursor<'_>, TokenRangeError> {
        TokenCursor::new(self, range)
    }

    /// Register one checked segmented token sequence in this source owner.
    pub fn try_register_token_sequence(
        &mut self,
        ranges: &[TokenRange],
    ) -> Result<TokenSequenceId, TokenSequenceError> {
        self.sequence_store.try_push(ranges)
    }

    /// Borrow one source-local segmented token sequence.
    pub fn token_sequence(
        &self,
        id: TokenSequenceId,
    ) -> Result<TokenSequenceView<'_>, TokenSequenceError> {
        TokenSequenceView::new(self, id)
    }
    pub(crate) fn rebind_source_identity(&mut self, source: SourceId) {
        self.source = source;
        self.numeric_literals.rebind_source_identity(source);
        self.sequence_store.rebind_source_identity(source);
        if let Some(table) = self.path_syntax.as_mut()
            && let Some(table) = Arc::get_mut(table)
        {
            table.rebind_source_identity(source);
        }
    }

    pub(crate) fn freeze_numeric_literals(&mut self) {
        self.numeric_literals.freeze();
        self.sequence_store.freeze();
    }

    pub(crate) fn attach_shared_path_syntax(&mut self, table: Arc<PathSyntaxTable>) {
        self.path_syntax = Some(table);
    }
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if self.numeric_literals.is_frozen() {
            panic!("numeric literal remapping was requested after the source publication freeze");
        }
        self.numeric_literals.remap_string_ids(remap);
        for shape in &mut self.shapes {
            shape.remap_string_ids(remap);
        }
    }
}
