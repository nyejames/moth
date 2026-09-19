//! Token definitions and source-location primitives for the frontend tokenizer.
//!
//! WHAT: defines token taxonomy, compact source shapes, and location metadata threaded through parsing.
//! WHY: every frontend stage past lexing depends on one canonical source-token model.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::numeric_text::store::{
    NumericLiteralId, NumericLiteralStore, NumericLiteralStoreError,
};
use crate::compiler_frontend::numeric_text::token::{
    NumericLiteralKind, NumericLiteralToken,
};
use crate::compiler_frontend::paths::path_syntax::{
    PathSyntax, PathSyntaxError, PathSyntaxId, PathSyntaxTable,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{
    FrozenStringTable, StringId, StringIdRemap, StringTable, StringTableResolver,
};

use std::iter::Peekable;
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

/// A checked zero-based index into one source's token arrays.
///
/// The packed representation is deliberately `u32`; conversion to `usize` happens only at the
/// indexing boundary and is never retained in source-owned token data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TokenIndex(u32);

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
    source: SourceId,
    start: TokenIndex,
    end: TokenIndex,
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
#[derive(Clone, Debug)]
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
    tokens: &'a SourceTokens,
    id: TokenSequenceId,
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
    OutOfBounds {
        index: u32,
        len: usize,
    },
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
    pub(crate) fn preflight_numeric(
        &self,
    ) -> Result<NumericLiteralId, SourceTokenBuildError> {
        self.preflight_next_token()?;
        self.next_numeric_id()
    }

    fn next_numeric_id(&self) -> Result<NumericLiteralId, SourceTokenBuildError> {
        NumericLiteralId::try_from_index(self.staged_numeric)
            .ok_or(SourceTokenBuildError::Capacity)
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
            return Err(SourceTokenBuildError::Invariant(CompilerError::compiler_error(
                "numeric literal handle did not match the staged canonical position",
            )));
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

    pub(crate) fn with_path_syntax(
        source: SourceId,
        path_syntax: PathSyntaxTable,
    ) -> Self {
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
        let numeric_id = self
            .builder
            .preflight_numeric()
            .map_err(test_shape_error)?;
        let kind = literal.kind;
        self.numeric_literals
            .try_push(literal)
            .map_err(|error| CompilerError::compiler_error(format!(
                "test numeric literal insertion failed: {error:?}"
            )))?;
        self.builder
            .push_numeric(
                TokenTag::NUMERIC_LITERAL,
                numeric_kind_flags(kind),
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
#[derive(Clone, Debug)]
pub struct SourceTokens {
    source: SourceId,
    shapes: Box<[TokenShape]>,
    spans: Box<[LocalSpan]>,
    numeric_literals: NumericLiteralStore,
    /// A path table is attached once its preparation owner reaches the immutable boundary.
    ///
    /// Direct source-store construction installs the shared immutable table without copying rows.
    path_syntax: Option<Arc<PathSyntaxTable>>,
    sequence_store: TokenSequenceStore,
    token_stats: TokenStats,
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
        self.shapes[index] = TokenShape::new(tag, 0, 0)
            .expect("the test corruption tag must accept zero flags");
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

/// A borrowed source token with typed, non-cloning payload views.
#[derive(Clone, Copy, Debug)]
pub struct TokenRef<'a> {
    tokens: &'a SourceTokens,
    index: TokenIndex,
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
        Self::with_capacity(
            source_code,
            file_id,
            entry_mode,
            extended_span_builder,
            0,
        )
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
        let span = self
            .current_local_span()
            .map_err(TokenEmitError::Span)?;
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

    pub(crate) fn emit_raw_string(
        &mut self,
        value: StringId,
    ) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::RAW_STRING_LITERAL, 0, value.index())
    }

    pub(crate) fn emit_char(&mut self, value: char) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::CHAR_LITERAL, 0, value as u32)
    }

    pub(crate) fn emit_bool(&mut self, value: bool) -> Result<TokenTag, TokenEmitError> {
        self.emit_payload(TokenTag::BOOL_LITERAL, 0, u32::from(value))
    }

    pub(crate) fn emit_path(&mut self, root: PathId) -> Result<TokenTag, TokenEmitError> {
        let span = self
            .current_local_span()
            .map_err(TokenEmitError::Span)?;
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
        let span = self
            .current_local_span()
            .map_err(TokenEmitError::Span)?;
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
/// Every value is explicit in the schema invocation below. It is independent of declaration
/// order so adding or reordering source variants cannot change retained diagnostic data.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TokenTag(u16);

/// Fixed source-token shape: one stable tag, reserved/semantic flags and one compact payload.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenShape {
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
    #[cfg(test)]
    pub(crate) const fn tag(self) -> TokenTag {
        self.tag
    }

    #[cfg(test)]
    pub(crate) const fn descriptor(self) -> TokenDescriptor {
        self.descriptor
    }

    #[cfg(test)]
    pub(crate) const fn allowed_flags(self) -> u16 {
        self.allowed_flags
    }

    #[cfg(test)]
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


            #[cfg(test)]
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

            #[cfg(test)]
            pub(crate) fn is_keyword(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_KEYWORD)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_word_operator(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_WORD_OPERATOR)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_literal(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_LITERAL)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_builtin_type(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_BUILTIN_TYPE)
                })
            }

            #[cfg(test)]
            pub(crate) fn is_delimiter(self) -> bool {
                self.schema().is_some_and(|schema| {
                    schema.has_class(TOKEN_CLASS_DELIMITER)
                })
            }

            /// Whether this tag counts toward the stats symbol bucket.
            ///
            /// WHAT: the schema-owned fact behind `TokenStats::symbols`.
            /// WHY: capacity seeds must read one taxonomy authority instead of a second
            ///      hand-maintained symbol table.
            pub(crate) fn is_stats_symbol(self) -> bool {
                self == Self::SYMBOL
            }

            /// Whether this tag counts toward the stats literal bucket.
            ///
            /// WHAT: the schema-owned fact behind `TokenStats::literals`. It covers the string,
            ///      raw-string, numeric, char, bool, and `none` value spellings that the legacy
            ///      classifier counted.
            /// WHY: capacity seeds must read one taxonomy authority instead of a second
            ///      hand-maintained literal table.
            pub(crate) fn is_stats_literal(self) -> bool {
                matches!(
                    self,
                    Self::STRING_SLICE_LITERAL
                        | Self::RAW_STRING_LITERAL
                        | Self::NUMERIC_LITERAL
                        | Self::CHAR_LITERAL
                        | Self::BOOL_LITERAL
                        | Self::NONE_LITERAL
                )
            }

            /// Whether this tag counts toward the stats operator bucket.
            ///
            /// WHAT: the schema-owned fact behind `TokenStats::operators`. It preserves the exact
            ///      legacy operator set, including expression-continuing punctuation (`->`),
            ///      word operators, postfix markers (`!`, `?`), `copy`, channels, `&`, and `=>`.
            /// WHY: capacity seeds must read one taxonomy authority instead of a second
            ///      hand-maintained operator table.
            pub(crate) fn is_stats_operator(self) -> bool {
                matches!(
                    self,
                    Self::ARROW
                        | Self::ADD
                        | Self::SUBTRACT
                        | Self::MULTIPLY
                        | Self::DIVIDE
                        | Self::MODULUS
                        | Self::INT_DIVIDE
                        | Self::EXPONENT
                        | Self::NEGATIVE
                        | Self::ADD_ASSIGN
                        | Self::SUBTRACT_ASSIGN
                        | Self::MULTIPLY_ASSIGN
                        | Self::DIVIDE_ASSIGN
                        | Self::MODULUS_ASSIGN
                        | Self::EXPONENT_ASSIGN
                        | Self::INT_DIVIDE_ASSIGN
                        | Self::LESS_THAN
                        | Self::LESS_THAN_OR_EQUAL
                        | Self::GREATER_THAN
                        | Self::GREATER_THAN_OR_EQUAL
                        | Self::IS
                        | Self::AND
                        | Self::OR
                        | Self::NOT
                        | Self::BANG
                        | Self::QUESTION_MARK
                        | Self::COPY
                        | Self::CHANNEL_SEND
                        | Self::CHANNEL_RECEIVE
                        | Self::AMPERSAND
                        | Self::FAT_ARROW
                )
            }

            #[cfg(test)]
            pub(crate) fn precedence(self) -> Option<u8> {
                self.schema().and_then(TokenSchema::precedence)
            }
        }

    };
}

const TOKEN_NUMERIC_FLAGS: u16 = 0b11;

token_schema! {
    (ModuleStart, MODULE_START, 1, "module start", Static, 0, 0, None),
    (Eof, EOF, 2, "end of file", Static, 0, TOKEN_CLASS_DELIMITER, None),
    (Export, EXPORT, 3, "`export`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Hash, HASH, 4, "`#`", Static, 0, 0, None),
    (Reactive, REACTIVE, 5, "`$`", Static, 0, 0, None),
    (Arrow, ARROW, 6, "`->`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, None),
    (
        Symbol(_),
        SYMBOL,
        7,
        "name",
        Symbol,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        StyleDirective(_),
        STYLE_DIRECTIVE,
        8,
        "style directive",
        StyleDirective,
        0,
        0,
        None
    ),
    (
        StringSliceLiteral(_),
        STRING_SLICE_LITERAL,
        9,
        "string literal",
        StringLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (Path(_), PATH, 10, "path", Path, 0, TOKEN_CLASS_OPERAND_START, None),
    (
        NumericLiteral(_),
        NUMERIC_LITERAL,
        11,
        "numeric literal",
        NumericLiteral,
        TOKEN_NUMERIC_FLAGS,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        CharLiteral(_),
        CHAR_LITERAL,
        12,
        "character literal",
        CharLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        RawStringLiteral(_),
        RAW_STRING_LITERAL,
        13,
        "raw string literal",
        RawStringLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        BoolLiteral(_),
        BOOL_LITERAL,
        14,
        "boolean literal",
        BoolLiteral,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        OpenCurly,
        OPEN_CURLY,
        15,
        "`{`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        CloseCurly,
        CLOSE_CURLY,
        16,
        "`}`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TypeParameterBracket,
        TYPE_PARAMETER_BRACKET,
        17,
        "`|`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        Newline,
        NEWLINE,
        18,
        "newline",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        End,
        END,
        19,
        "`;`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        StartTemplateBody,
        START_TEMPLATE_BODY,
        20,
        "`:`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        Comma,
        COMMA,
        21,
        "`,`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (Dot, DOT, 22, "`.`", Static, 0, TOKEN_CLASS_DELIMITER, None),
    (
        Colon,
        COLON,
        23,
        "`:`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        DoubleColon,
        DOUBLE_COLON,
        24,
        "`::`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        Assign,
        ASSIGN,
        25,
        "`=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        This,
        THIS,
        26,
        "`this`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (Must, MUST, 27, "`must`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        TraitThis,
        TRAIT_THIS,
        28,
        "`This`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (
        OpenParenthesis,
        OPEN_PARENTHESIS,
        29,
        "`(`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CONTINUES_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        CloseParenthesis,
        CLOSE_PARENTHESIS,
        30,
        "`)`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (As, AS, 31, "`as`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Type, TYPE, 32, "`type`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Of, OF, 33, "`of`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        Variadic,
        VARIADIC,
        34,
        "`..`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        Mutable,
        MUTABLE,
        35,
        "`~`",
        Static,
        0,
        TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        DatatypeNone,
        DATATYPE_NONE,
        36,
        "`None` type",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        NoneLiteral,
        NONE_LITERAL,
        37,
        "`none`",
        Static,
        0,
        TOKEN_CLASS_LITERAL | TOKEN_CLASS_CAN_END_EXPRESSION | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        DatatypeInt,
        DATATYPE_INT,
        38,
        "`Int`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeFloat,
        DATATYPE_FLOAT,
        39,
        "`Float`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeBool,
        DATATYPE_BOOL,
        40,
        "`Bool`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeTrue,
        DATATYPE_TRUE,
        41,
        "`True`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeFalse,
        DATATYPE_FALSE,
        42,
        "`False`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeString,
        DATATYPE_STRING,
        43,
        "`String`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        DatatypeChar,
        DATATYPE_CHAR,
        44,
        "`Char`",
        Static,
        0,
        TOKEN_CLASS_BUILTIN_TYPE,
        None
    ),
    (
        Bang,
        BANG,
        45,
        "`!`",
        Static,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        QuestionMark,
        QUESTION_MARK,
        46,
        "`?`",
        Static,
        0,
        TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (Negative, NEGATIVE, 47, "unary `-`", Static, 0, 0, Some(6)),
    (Exponent, EXPONENT, 48, "`^`", Static, 0, 0, Some(5)),
    (Multiply, MULTIPLY, 49, "`*`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (Divide, DIVIDE, 50, "`/`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (Modulus, MODULUS, 51, "`%`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (IntDivide, INT_DIVIDE, 52, "`//`", Static, 0, TOKEN_CLASS_CONTINUES_EXPRESSION, Some(4)),
    (
        ExponentAssign,
        EXPONENT_ASSIGN,
        53,
        "`^=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        MultiplyAssign,
        MULTIPLY_ASSIGN,
        54,
        "`*=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        DivideAssign,
        DIVIDE_ASSIGN,
        55,
        "`/=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        ModulusAssign,
        MODULUS_ASSIGN,
        56,
        "`%=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        IntDivideAssign,
        INT_DIVIDE_ASSIGN,
        57,
        "`//=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        Add,
        ADD,
        58,
        "`+`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(3)
    ),
    (
        Subtract,
        SUBTRACT,
        59,
        "`-`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(3)
    ),
    (
        AddAssign,
        ADD_ASSIGN,
        60,
        "`+=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        SubtractAssign,
        SUBTRACT_ASSIGN,
        61,
        "`-=`",
        Static,
        0,
        TOKEN_CLASS_ASSIGNMENT | TOKEN_CLASS_CONTINUES_EXPRESSION,
        None
    ),
    (
        Not,
        NOT,
        62,
        "`not`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR,
        Some(6)
    ),
    (
        Is,
        IS,
        63,
        "`is`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR | TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        LessThan,
        LESS_THAN,
        64,
        "`<`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        LessThanOrEqual,
        LESS_THAN_OR_EQUAL,
        65,
        "`<=`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        GreaterThan,
        GREATER_THAN,
        66,
        "`>`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        GreaterThanOrEqual,
        GREATER_THAN_OR_EQUAL,
        67,
        "`>=`",
        Static,
        0,
        TOKEN_CLASS_CONTINUES_EXPRESSION,
        Some(2)
    ),
    (
        And,
        AND,
        68,
        "`and`",
        Static,
        0,
        TOKEN_CLASS_WORD_OPERATOR,
        Some(1)
    ),
    (Or, OR, 69, "`or`", Static, 0, TOKEN_CLASS_WORD_OPERATOR, Some(0)),
    (If, IF, 70, "`if`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Else, ELSE, 71, "`else`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Return, RETURN, 72, "`return`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        ReturnBang,
        RETURN_BANG,
        73,
        "`return!`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (Catch, CATCH, 74, "`catch`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Then, THEN, 75, "`then`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Checked, CHECKED, 76, "`checked`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Async, ASYNC, 77, "`async`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Cast, CAST, 78, "`cast`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        CastBang,
        CAST_BANG,
        79,
        "`cast!`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (Assert, ASSERT, 80, "`assert`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Loop, LOOP, 81, "`loop`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (By, BY, 82, "`by`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (Break, BREAK, 83, "`break`", Static, 0, TOKEN_CLASS_KEYWORD, None),
    (
        Continue,
        CONTINUE,
        84,
        "`continue`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        None
    ),
    (
        ExclusiveRange,
        EXCLUSIVE_RANGE,
        85,
        "`to`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD,
        Some(6)
    ),
    (Ampersand, AMPERSAND, 86, "`&`", Static, 0, 0, None),
    (
        FatArrow,
        FAT_ARROW,
        87,
        "`=>`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (Wildcard, WILDCARD, 88, "`_`", Static, 0, 0, None),
    (
        Copy,
        COPY,
        89,
        "`copy`",
        Static,
        0,
        TOKEN_CLASS_KEYWORD | TOKEN_CLASS_OPERAND_START,
        None
    ),
    (
        TemplateClose,
        TEMPLATE_CLOSE,
        90,
        "`]`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER | TOKEN_CLASS_CAN_END_EXPRESSION,
        None
    ),
    (
        TemplateHead,
        TEMPLATE_HEAD,
        91,
        "`[`",
        Static,
        0,
        TOKEN_CLASS_DELIMITER,
        None
    ),
    (
        ChannelSend,
        CHANNEL_SEND,
        92,
        "`>>`",
        Static,
        0,
        0,
        None
    ),
    (
        ChannelReceive,
        CHANNEL_RECEIVE,
        93,
        "`<<`",
        Static,
        0,
        0,
        None
    ),
    (Yield, YIELD, 94, "`yield`", Static, 0, TOKEN_CLASS_KEYWORD, None),
}

impl TokenShape {
    /// Construct a shape only when its flags are valid for the selected token tag.
    ///
    /// Payload validation is performed by [`Self::from_raw_parts`] and the typed accessors. This
    /// constructor remains a low-level packing primitive for taxonomy tests and source adapters.
    #[cfg(test)]
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
            TokenDescriptorPayload::CharLiteral => flags == 0 && char::from_u32(data).is_some(),
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::RawStringLiteral => flags == 0,
        }
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
    fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if self.string_id().is_some() {
            self.data = remap.get(StringId::from_index(self.data)).index();
        }
    }
}
pub(crate) const fn numeric_kind_flags(kind: NumericLiteralKind) -> u16 {
    match kind {
        NumericLiteralKind::WholeNumber => 0,
        NumericLiteralKind::DecimalPoint => 1,
        NumericLiteralKind::Exponent => 2,
    }
}



#[cfg(test)]
#[path = "tests/tokens_remap_tests.rs"]
mod tokens_remap_tests;

#[cfg(test)]
#[path = "tests/token_cursor_tests.rs"]
mod token_cursor_tests;
#[cfg(test)]
#[path = "tests/token_taxonomy_tests.rs"]
mod token_taxonomy_tests;
