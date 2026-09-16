//! Token definitions and source-location primitives for the frontend tokenizer.
//!
//! WHAT: defines token kinds, token records, and the location metadata threaded through parsing.
//! WHY: every frontend stage past lexing depends on one canonical token and location model.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::arena::TokenStats;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::numeric_text::store::{NumericLiteralId, NumericLiteralStore};
use crate::compiler_frontend::numeric_text::token::{NumericLiteralKind, NumericLiteralToken};
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
#[cfg(test)]
use crate::compiler_frontend::symbols::path_interner::PathIdRemap;
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

    pub const fn id(self) -> TokenSequenceId {
        self.id
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
    #[cfg(test)]
    MissingPathTable,
    MalformedNumericHandle,
    MalformedPathHandle,
}

/// The immutable source-owned shape/span arrays and their typed cold stores.
///
/// `TokenKind` remains in [`FileTokens`] only as a private compatibility adapter for parsers that
/// still consume legacy token values. This owner is the canonical representation used by cursor
/// consumers: shapes and spans are frozen dense SoA arrays, while numeric and path rows stay
/// source-local.
#[derive(Clone, Debug)]
pub struct SourceTokens {
    source: SourceId,
    shapes: Box<[TokenShape]>,
    spans: Box<[LocalSpan]>,
    numeric_literals: NumericLiteralStore,
    /// A path table is attached once its preparation owner reaches the immutable boundary.
    ///
    /// Direct source-store construction installs it immediately. The compatibility `FileTokens`
    /// adapter leaves this absent while its preparing path table is mutable, then shares the
    /// frozen allocation without copying rows.
    path_syntax: Option<Arc<PathSyntaxTable>>,
    sequence_store: TokenSequenceStore,
    token_stats: TokenStats,
}

impl SourceTokens {
    /// Build the canonical source arrays from a legacy token vector.
    ///
    /// The returned numeric handle lane is retained only by the private `FileTokens` adapter.
    /// When a shared table is supplied, every path handle is additionally checked against its
    /// row span so malformed or cross-source handles cannot enter the immutable owner through
    /// this adapter. Table-less construction keeps `path_syntax` absent until ordinary
    /// publication attaches the frozen allocation without copying rows.
    pub(crate) fn from_legacy_tokens(
        source: SourceId,
        tokens: &[Token],
        numeric_literals: NumericLiteralStore,
        numeric_literal_ids: Option<&[Option<NumericLiteralId>]>,
        path_syntax_owner: Option<Arc<PathSyntaxTable>>,
        token_stats: TokenStats,
    ) -> Result<(Self, Vec<Option<NumericLiteralId>>), CompilerError> {
        if numeric_literals.owner_source().is_none() && !numeric_literals.is_empty() {
            return Err(CompilerError::compiler_error(
                "legacy source token numeric store has records but no source identity",
            ));
        }
        if let Some(owner) = numeric_literals.owner_source()
            && owner != source
        {
            return Err(CompilerError::compiler_error(
                "legacy source token numeric store does not match its source identity",
            ));
        }
        if let Some(table) = path_syntax_owner.as_deref() {
            table.validate_file_owned_locations(source).map_err(|_| {
                CompilerError::compiler_error(
                    "legacy source token path table does not match its source identity",
                )
            })?;
        }
        let numeric_literal_ids = match numeric_literal_ids {
            Some(ids) if ids.len() == tokens.len() => ids.to_vec(),
            Some(_) => {
                return Err(CompilerError::compiler_error(
                    "legacy token numeric handles do not align with token positions",
                ));
            }
            None => numeric_ids_for_staged_store(tokens, &numeric_literals),
        };
        for (index, token) in tokens.iter().enumerate() {
            let numeric_id = numeric_literal_ids[index];
            let is_numeric = matches!(token.kind, TokenKind::NumericLiteral(_));
            match (is_numeric, numeric_id) {
                (true, Some(id)) => {
                    numeric_literals
                        .try_get_for_source(id, source)
                        .map_err(|_| {
                            CompilerError::compiler_error(format!(
                                "legacy token numeric handle at staged position {index} is invalid"
                            ))
                        })?;
                }
                (true, None) => {
                    return Err(CompilerError::compiler_error(format!(
                        "legacy numeric token at index {index} is missing its side-store handle"
                    )));
                }
                (false, Some(_)) => {
                    return Err(CompilerError::compiler_error(format!(
                        "legacy non-numeric token at index {index} carries a numeric handle"
                    )));
                }
                (false, None) => {}
            }
        }
        let mut shapes = Vec::with_capacity(tokens.len());
        let mut spans = Vec::with_capacity(tokens.len());
        // Stats accumulate from the canonical shapes packed here, so every constructed owner
        // carries the same tag-authority counts. A non-default seed is preserved for callers
        // that already counted alongside construction; fresh construction starts from default
        // and fills each bucket from the packed shapes below.
        let mut token_stats = token_stats;
        let count_from_shapes = token_stats == TokenStats::default();
        for (index, token) in tokens.iter().enumerate() {
            let numeric_id = numeric_literal_ids[index].unwrap_or(NumericLiteralId::NONE);
            let shape = TokenShape::from_token_kind_with_numeric_id(&token.kind, numeric_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "legacy token at index {index} has a malformed compact shape"
                    ))
                })?;
            if let (Some(table), Some(path_id)) =
                (path_syntax_owner.as_deref(), shape.path_syntax_id())
            {
                table
                    .try_path_for_token(path_id, SourceSpan::new(source, token.span))
                    .map_err(|_| {
                        CompilerError::compiler_error(format!(
                            "legacy token path handle at index {index} is invalid"
                        ))
                    })?;
            }
            if count_from_shapes {
                token_stats.accumulate_shape(shape);
            }
            shapes.push(shape);
            spans.push(token.span);
        }
        let sequence_store = TokenSequenceStore::new(source, tokens.len());
        let owner = Self {
            source,
            shapes: shapes.into_boxed_slice(),
            spans: spans.into_boxed_slice(),
            numeric_literals,
            path_syntax: path_syntax_owner,
            sequence_store,
            token_stats,
        };

        owner.validate_structure()?;
        Ok((owner, numeric_literal_ids))
    }

    /// Build an immutable source store directly from a legacy token vector.
    ///
    /// The vector is consumed as a construction adapter; only compact shapes/spans survive in
    /// the returned owner. Numeric and path records are moved without cloning cold payload rows.
    #[cfg(test)]
    pub fn try_from_tokens(
        source: SourceId,
        tokens: Vec<Token>,
        numeric_literals: NumericLiteralStore,
        path_syntax: PathSyntaxTable,
        token_stats: TokenStats,
    ) -> Result<Self, CompilerError> {
        path_syntax.validate_file_tokens(&tokens, source, "source token owner")?;
        let path_owner = Arc::new(path_syntax);
        let (mut owner, _) = Self::from_legacy_tokens(
            source,
            &tokens,
            numeric_literals,
            None,
            Some(path_owner),
            token_stats,
        )?;
        owner.numeric_literals.freeze();
        owner.sequence_store.freeze();
        if let Some(table) = owner.path_syntax.as_mut()
            && let Some(table) = Arc::get_mut(table)
        {
            table.freeze();
        }
        Ok(owner)
    }

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
        if self.shapes.len() > u32::MAX as usize {
            return Err(CompilerError::compiler_error(
                "source token arrays exceed their checked u32 index domain",
            ));
        }
        Ok(())
    }

    fn validate_range(&self, range: TokenRange) -> Result<(), TokenRangeError> {
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

    /// Materialize one checked contiguous range directly from this canonical owner.
    ///
    /// Parser adapters use this only at an explicit handoff boundary. The returned vector is
    /// transient compatibility data; canonical shapes, spans and cold stores remain owned here.
    pub(crate) fn materialize_range(&self, range: TokenRange) -> Result<Vec<Token>, CompilerError> {
        let mut cursor = self.cursor(range).map_err(|error| {
            CompilerError::compiler_error(format!("token range materialization failed: {error:?}"))
        })?;
        let mut tokens = Vec::with_capacity(range.len() as usize);
        while let Some(token_ref) = cursor.advance() {
            let is_eof = token_ref.is_eof();
            let kind = token_ref.to_token_kind().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "canonical token payload could not be materialized: {error:?}"
                ))
            })?;
            tokens.push(Token::new(kind, token_ref.span()));
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    /// Materialize one checked segmented sequence directly from this canonical owner.
    ///
    /// The returned vector is transient parser compatibility data; the sequence store
    /// remains owned here.
    #[cfg(test)]
    pub(crate) fn materialize_token_sequence(
        &self,
        id: TokenSequenceId,
    ) -> Result<Vec<Token>, CompilerError> {
        let view = self.token_sequence(id).map_err(|error| {
            CompilerError::compiler_error(format!(
                "token sequence view construction failed: {error:?}"
            ))
        })?;
        let cursor = view.cursor().map_err(|error| {
            CompilerError::compiler_error(format!(
                "token sequence cursor construction failed: {error:?}"
            ))
        })?;
        let mut tokens = Vec::with_capacity(view.len());
        let mut cursor = cursor;
        while let Some(token_ref) = cursor.advance() {
            let is_eof = token_ref.is_eof();
            let kind = token_ref.to_token_kind().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "canonical token payload could not be materialized: {error:?}"
                ))
            })?;
            tokens.push(Token::new(kind, token_ref.span()));
            if is_eof {
                break;
            }
        }
        Ok(tokens)
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
    /// `Ok(None)` means this token carries no path payload. `MissingPathTable` means the
    /// deferred publication table has not been attached yet; that lifecycle gap closes at the
    /// ordinary `freeze_path_syntax` boundary.
    #[cfg(test)]
    pub fn path_syntax(
        self,
    ) -> Result<Option<&'a crate::compiler_frontend::paths::path_syntax::PathSyntax>, TokenViewError>
    {
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

    pub(crate) fn string_id(self) -> Option<StringId> {
        self.shape().string_id()
    }

    /// Stable tag for this token without cloning cold payloads.
    ///
    /// 3F1 header classification reads this instead of `TokenKind` matches.
    pub(crate) fn tag(self) -> TokenTag {
        self.shape().tag()
    }

    /// Dense path handle for this token, if it carries one.
    ///
    /// Header-stage callers resolve the row through the `FilePathSyntax` lifecycle
    /// table while the canonical store is still `Preparing`/`Deferred`.
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
    /// Materialize this short-lived canonical view at an explicit compatibility boundary.
    ///
    /// Declaration and signature parsers inspect the stable tag and typed payloads directly, but
    /// their diagnostics still carry the legacy `TokenKind` payload shape. This conversion is
    /// deliberately local and fallible: malformed compact payloads stay infrastructure failures
    /// instead of becoming fabricated source tokens.
    pub(crate) fn to_token_kind(self) -> Result<TokenKind, TokenViewError> {
        let tag = self.tag();
        let kind = match tag {
            TokenTag::MODULE_START => TokenKind::ModuleStart,
            TokenTag::EOF => TokenKind::Eof,
            TokenTag::EXPORT => TokenKind::Export,
            TokenTag::HASH => TokenKind::Hash,
            TokenTag::REACTIVE => TokenKind::Reactive,
            TokenTag::ARROW => TokenKind::Arrow,
            TokenTag::SYMBOL => TokenKind::Symbol(
                self.string_id()
                    .ok_or(TokenViewError::MalformedNumericHandle)?,
            ),
            TokenTag::STYLE_DIRECTIVE => TokenKind::StyleDirective(
                self.string_id()
                    .ok_or(TokenViewError::MalformedNumericHandle)?,
            ),
            TokenTag::STRING_SLICE_LITERAL => TokenKind::StringSliceLiteral(
                self.string_id()
                    .ok_or(TokenViewError::MalformedNumericHandle)?,
            ),
            TokenTag::PATH => TokenKind::Path(
                self.path_syntax_id()
                    .ok_or(TokenViewError::MalformedPathHandle)?,
            ),
            TokenTag::NUMERIC_LITERAL => TokenKind::NumericLiteral(
                self.numeric_literal()?
                    .ok_or(TokenViewError::MalformedNumericHandle)?
                    .clone(),
            ),
            TokenTag::CHAR_LITERAL => TokenKind::CharLiteral(
                self.char_value()
                    .ok_or(TokenViewError::MalformedNumericHandle)?,
            ),
            TokenTag::RAW_STRING_LITERAL => TokenKind::RawStringLiteral(
                self.string_id()
                    .ok_or(TokenViewError::MalformedNumericHandle)?,
            ),
            TokenTag::BOOL_LITERAL => TokenKind::BoolLiteral(
                self.bool_value()
                    .ok_or(TokenViewError::MalformedNumericHandle)?,
            ),
            TokenTag::OPEN_CURLY => TokenKind::OpenCurly,
            TokenTag::CLOSE_CURLY => TokenKind::CloseCurly,
            TokenTag::TYPE_PARAMETER_BRACKET => TokenKind::TypeParameterBracket,
            TokenTag::NEWLINE => TokenKind::Newline,
            TokenTag::END => TokenKind::End,
            TokenTag::START_TEMPLATE_BODY => TokenKind::StartTemplateBody,
            TokenTag::COMMA => TokenKind::Comma,
            TokenTag::DOT => TokenKind::Dot,
            TokenTag::COLON => TokenKind::Colon,
            TokenTag::DOUBLE_COLON => TokenKind::DoubleColon,
            TokenTag::ASSIGN => TokenKind::Assign,
            TokenTag::THIS => TokenKind::This,
            TokenTag::MUST => TokenKind::Must,
            TokenTag::TRAIT_THIS => TokenKind::TraitThis,
            TokenTag::OPEN_PARENTHESIS => TokenKind::OpenParenthesis,
            TokenTag::CLOSE_PARENTHESIS => TokenKind::CloseParenthesis,
            TokenTag::AS => TokenKind::As,
            TokenTag::TYPE => TokenKind::Type,
            TokenTag::OF => TokenKind::Of,
            TokenTag::VARIADIC => TokenKind::Variadic,
            TokenTag::MUTABLE => TokenKind::Mutable,
            TokenTag::DATATYPE_NONE => TokenKind::DatatypeNone,
            TokenTag::NONE_LITERAL => TokenKind::NoneLiteral,
            TokenTag::DATATYPE_INT => TokenKind::DatatypeInt,
            TokenTag::DATATYPE_FLOAT => TokenKind::DatatypeFloat,
            TokenTag::DATATYPE_BOOL => TokenKind::DatatypeBool,
            TokenTag::DATATYPE_TRUE => TokenKind::DatatypeTrue,
            TokenTag::DATATYPE_FALSE => TokenKind::DatatypeFalse,
            TokenTag::DATATYPE_STRING => TokenKind::DatatypeString,
            TokenTag::DATATYPE_CHAR => TokenKind::DatatypeChar,
            TokenTag::BANG => TokenKind::Bang,
            TokenTag::QUESTION_MARK => TokenKind::QuestionMark,
            TokenTag::NEGATIVE => TokenKind::Negative,
            TokenTag::EXPONENT => TokenKind::Exponent,
            TokenTag::MULTIPLY => TokenKind::Multiply,
            TokenTag::DIVIDE => TokenKind::Divide,
            TokenTag::MODULUS => TokenKind::Modulus,
            TokenTag::INT_DIVIDE => TokenKind::IntDivide,
            TokenTag::EXPONENT_ASSIGN => TokenKind::ExponentAssign,
            TokenTag::MULTIPLY_ASSIGN => TokenKind::MultiplyAssign,
            TokenTag::DIVIDE_ASSIGN => TokenKind::DivideAssign,
            TokenTag::MODULUS_ASSIGN => TokenKind::ModulusAssign,
            TokenTag::INT_DIVIDE_ASSIGN => TokenKind::IntDivideAssign,
            TokenTag::ADD => TokenKind::Add,
            TokenTag::SUBTRACT => TokenKind::Subtract,
            TokenTag::ADD_ASSIGN => TokenKind::AddAssign,
            TokenTag::SUBTRACT_ASSIGN => TokenKind::SubtractAssign,
            TokenTag::NOT => TokenKind::Not,
            TokenTag::IS => TokenKind::Is,
            TokenTag::LESS_THAN => TokenKind::LessThan,
            TokenTag::LESS_THAN_OR_EQUAL => TokenKind::LessThanOrEqual,
            TokenTag::GREATER_THAN => TokenKind::GreaterThan,
            TokenTag::GREATER_THAN_OR_EQUAL => TokenKind::GreaterThanOrEqual,
            TokenTag::AND => TokenKind::And,
            TokenTag::OR => TokenKind::Or,
            TokenTag::IF => TokenKind::If,
            TokenTag::ELSE => TokenKind::Else,
            TokenTag::RETURN => TokenKind::Return,
            TokenTag::RETURN_BANG => TokenKind::ReturnBang,
            TokenTag::CATCH => TokenKind::Catch,
            TokenTag::THEN => TokenKind::Then,
            TokenTag::CHECKED => TokenKind::Checked,
            TokenTag::ASYNC => TokenKind::Async,
            TokenTag::CAST => TokenKind::Cast,
            TokenTag::CAST_BANG => TokenKind::CastBang,
            TokenTag::ASSERT => TokenKind::Assert,
            TokenTag::LOOP => TokenKind::Loop,
            TokenTag::BY => TokenKind::By,
            TokenTag::BREAK => TokenKind::Break,
            TokenTag::CONTINUE => TokenKind::Continue,
            TokenTag::EXCLUSIVE_RANGE => TokenKind::ExclusiveRange,
            TokenTag::AMPERSAND => TokenKind::Ampersand,
            TokenTag::FAT_ARROW => TokenKind::FatArrow,
            TokenTag::WILDCARD => TokenKind::Wildcard,
            TokenTag::COPY => TokenKind::Copy,
            TokenTag::TEMPLATE_CLOSE => TokenKind::TemplateClose,
            TokenTag::TEMPLATE_HEAD => TokenKind::TemplateHead,
            TokenTag::CHANNEL_SEND => TokenKind::ChannelSend,
            TokenTag::CHANNEL_RECEIVE => TokenKind::ChannelReceive,
            TokenTag::YIELD => TokenKind::Yield,
            _ => return Err(TokenViewError::MalformedNumericHandle),
        };
        Ok(kind)
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

    /// Return the parser-facing length while retaining raw source ranges in `range`.
    ///
    /// Contiguous cursors report their active range end. Segmented cursors report the cached
    /// total logical length.
    pub(crate) fn parser_length(self) -> usize {
        match self.bounds {
            TokenCursorBounds::Contiguous(range) => range.end().index(),
            TokenCursorBounds::Segmented(_) => self.logical_length,
        }
    }

    /// Materialise the current canonical token for an explicit compatibility boundary.
    pub(crate) fn current_token_owned(self) -> Result<Option<Token>, CompilerError> {
        self.current()
            .map(|token| {
                token
                    .to_token_kind()
                    .map(|kind| Token::new(kind, token.span()))
                    .map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "canonical token payload could not be materialised: {error:?}"
                        ))
                    })
            })
            .transpose()
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
    /// Construct a contiguous cursor at a checked position inside `range`.
    ///
    /// The position is canonical (source-local), rather than an adapter-relative index. This is
    /// the transient handoff used when a bounded compatibility parser resumes from its current
    /// legacy-vector position.
    fn from_range_position(
        tokens: &'a SourceTokens,
        range: TokenRange,
        position: TokenIndex,
    ) -> Result<Self, TokenRangeError> {
        tokens.validate_range(range)?;
        if position < range.start || position > range.end {
            return Err(TokenRangeError::OutOfBounds {
                start: position.raw(),
                end: position.raw(),
                len: range.end.index(),
            });
        }
        Ok(Self {
            tokens,
            bounds: TokenCursorBounds::Contiguous(range),
            segment_index: 0,
            next: position,
            logical_position: 0,
            logical_length: 0,
            segment_start_position: 0,
            previous_segment_index: None,
            next_segment_index: None,
        })
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

    /// Return the number of materialised tokens consumed by this cursor.
    ///
    /// Contiguous positions are canonical indexes and therefore require the caller's range
    /// metadata to subtract its start. Segmented positions count sequence entries and intentionally
    /// skip source gaps.
    fn compatibility_position(self) -> Result<usize, TokenSequenceError> {
        match self.bounds {
            TokenCursorBounds::Contiguous(_) => Ok(self.next.index()),
            TokenCursorBounds::Segmented(_) => Ok(self.logical_position),
        }
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
        match self.bounds {
            TokenCursorBounds::Contiguous(range) => {
                if index < range.start().index() || index >= range.end().index() {
                    return None;
                }
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
        if position > self.logical_length {
            return Err(TokenSequenceError::OutOfRange {
                raw: u32::try_from(position).unwrap_or(u32::MAX),
                len: self.logical_length,
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

    /// Return the position of the cursor within its active compatibility view.
    ///
    /// Contiguous ranges use offsets from their range start; segmented sequences count logical
    /// tokens across all prior ranges. This is the bridge for transient parser adapters whose
    /// payload IDs may be remapped while spans remain canonical.
    pub(crate) fn compatibility_position_from_start(self) -> Result<usize, CompilerError> {
        match self.bounds {
            TokenCursorBounds::Contiguous(range) => self
                .next
                .index()
                .checked_sub(range.start().index())
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "canonical cursor position precedes its active range start",
                    )
                }),
            TokenCursorBounds::Segmented(_) => self.compatibility_position().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "segmented cursor compatibility position was invalid: {error:?}"
                ))
            }),
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
                        };
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

    /// Create a nested cursor after proving that the child range is inside the active segment.
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
        Self::new(self.tokens, range)
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

    #[cfg(test)]
    fn permanent_substream(&self) -> Self {
        match self {
            // Header syntax does not inspect retained body tokens while parsing the file. Keep
            // the body deferred so the prepared-file owner remains the sole mutable table owner.
            Self::Preparing(_) | Self::Deferred => Self::Deferred,
            Self::Shared(table) => Self::Shared(Arc::clone(table)),
        }
    }

    fn shared_table_for_source_tokens(&self) -> Option<Arc<PathSyntaxTable>> {
        match self {
            Self::Shared(table) => Some(Arc::clone(table)),
            Self::Preparing(_) | Self::Deferred => None,
        }
    }

    pub(crate) fn frozen_substream(&self) -> Result<Self, CompilerError> {
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

    /// Attach after a whole-file preflight has proven this stream is deferred.
    ///
    /// This intentionally has no fallible branch. `FileFrontendPrepareOutput` first checks every
    /// retained header, then changes the file owner to frozen and attaches all parser adapters in
    /// one non-failing commit section. Adding an `Arc` clone never copies table rows or enables COW.
    #[cfg(test)]
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

/// Provenance retained by a transient parser adapter.
///
/// `Unbounded` is the explicit compatibility-only path used by expression parsers over copied
/// vectors. Bounded declaration adapters retain an `Arc` to the one canonical owner plus the
/// checked range/sequence that produced their vector; no second `SourceTokens` is constructed.
#[derive(Clone, Debug)]
enum FileTokenAdapterMetadata {
    Unbounded,
    Contiguous {
        source_tokens: Arc<SourceTokens>,
        range: TokenRange,
        /// Whether the compatibility vector carries one synthetic EOF after the retained range.
        ///
        /// Expression parsers need a stable sentinel even when the source-owned range stops at a
        /// declaration/member delimiter. The sentinel is adapter data only; canonical positions
        /// remain bounded by `range`.
        synthetic_trailing_eof: bool,
    },
    Sequence {
        source_tokens: Arc<SourceTokens>,
        sequence: TokenSequenceId,
    },
}

/// Explicit canonical-vs-adapter token ownership for one `FileTokens` stream.
///
/// WHAT: the top-level lexer output owns the sole canonical `SourceTokens` SoA
///       (shapes/spans plus numeric/path cold stores) for its `SourceId`.
///       Bounded parser adapters retain only an `Arc` to that owner, their checked provenance,
///       ephemeral `Token` vector, numeric side-store lane, and path lifecycle shell.
/// WHY: duplicate `SourceTokens` owners for the same `SourceId` make source-qualified ranges
///      ambiguous. Compatibility adapters therefore share the canonical allocation explicitly
///      for one transient parser handoff, while unbounded expression adapters retain no owner.
#[derive(Clone, Debug)]
enum FileTokenOwner {
    /// Sole canonical SoA owner for its source construction.
    Canonical(Arc<SourceTokens>),
    /// Bounded or compatibility parser adapter without owned shape/span arrays.
    Adapter {
        numeric_literals: NumericLiteralStore,
        metadata: FileTokenAdapterMetadata,
    },
}

impl FileTokenOwner {
    #[cfg(test)]
    fn numeric_literal_store(&self) -> &NumericLiteralStore {
        match self {
            Self::Canonical(owner) => owner.numeric_literal_store(),
            Self::Adapter {
                numeric_literals, ..
            } => numeric_literals,
        }
    }

    #[cfg(test)]
    fn remap_owner_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            Self::Canonical(owner) => Arc::get_mut(owner)
                .expect("canonical source owner must be uniquely mutable before adapter handoff")
                .remap_string_ids(remap),
            Self::Adapter {
                numeric_literals, ..
            } => numeric_literals.remap_string_ids(remap),
        }
    }

    fn rebind_owner_identity(&mut self, source: SourceId) {
        match self {
            Self::Canonical(owner) => Arc::get_mut(owner)
                .expect("canonical source owner must be uniquely mutable before adapter handoff")
                .rebind_source_identity(source),
            Self::Adapter {
                numeric_literals, ..
            } => numeric_literals.rebind_source_identity(source),
        }
    }

    fn freeze_owner_numeric_literals(&mut self) {
        match self {
            Self::Canonical(owner) => Arc::get_mut(owner)
                .expect("canonical source owner must be uniquely mutable before adapter handoff")
                .freeze_numeric_literals(),
            Self::Adapter {
                numeric_literals, ..
            } => numeric_literals.freeze(),
        }
    }

    #[cfg(test)]
    fn attach_owner_shared_path_syntax(&mut self, table: Arc<PathSyntaxTable>) {
        match self {
            Self::Canonical(owner) => Arc::get_mut(owner)
                .expect("canonical source owner must be uniquely mutable before adapter handoff")
                .attach_shared_path_syntax(table),
            // Adapters share the lifecycle shell handle only; they own no SoA table slot.
            Self::Adapter { .. } => {}
        }
    }

    fn as_canonical(&self) -> Result<&SourceTokens, CompilerError> {
        match self {
            Self::Canonical(owner) => Ok(owner.as_ref()),
            Self::Adapter { .. } => Err(CompilerError::compiler_error(
                "parser adapter stream owns no canonical source-token store",
            )),
        }
    }

    fn canonical_arc(&self) -> Result<Arc<SourceTokens>, CompilerError> {
        match self {
            Self::Canonical(owner) => Ok(Arc::clone(owner)),
            Self::Adapter { metadata, .. } => match metadata {
                FileTokenAdapterMetadata::Unbounded => Err(CompilerError::compiler_error(
                    "token adapter has no canonical source-token provenance",
                )),
                FileTokenAdapterMetadata::Contiguous { source_tokens, .. }
                | FileTokenAdapterMetadata::Sequence { source_tokens, .. } => {
                    Ok(Arc::clone(source_tokens))
                }
            },
        }
    }

    fn canonical_source_tokens(&self) -> Result<&SourceTokens, CompilerError> {
        match self {
            Self::Canonical(owner) => Ok(owner.as_ref()),
            Self::Adapter { metadata, .. } => match metadata {
                FileTokenAdapterMetadata::Unbounded => Err(CompilerError::compiler_error(
                    "token adapter has no canonical source-token provenance",
                )),
                FileTokenAdapterMetadata::Contiguous { source_tokens, .. }
                | FileTokenAdapterMetadata::Sequence { source_tokens, .. } => {
                    Ok(source_tokens.as_ref())
                }
            },
        }
    }

    fn as_canonical_mut(&mut self) -> Result<&mut SourceTokens, CompilerError> {
        match self {
            Self::Canonical(owner) => Arc::get_mut(owner).ok_or_else(|| {
                CompilerError::compiler_error(
                    "canonical source token owner is shared during a mutable lifecycle transition",
                )
            }),
            Self::Adapter { .. } => Err(CompilerError::compiler_error(
                "parser adapter stream owns no canonical source-token store",
            )),
        }
    }

    const fn is_canonical(&self) -> bool {
        matches!(self, Self::Canonical(_))
    }
}

/// Logical file identity shared by parser-adapter constructors.
///
/// WHAT: groups the source path, source identity, and canonical OS path that every adapter
/// construction needs together.
/// WHY: keeping them together keeps `with_adapter_path_syntax_and_numeric_store` under the
/// argument-count lint without changing validation or metadata.
struct AdapterIdentity {
    src_path: PathId,
    file_id: SourceId,
    canonical_os_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct FileTokens {
    /// Canonical owner exactly when this stream constructed its source; otherwise an adapter that
    /// may retain checked range/sequence provenance and an `Arc` to that canonical source. The
    /// parser compatibility vector remains on this shell, while retained headers keep only source
    /// ranges or sequence IDs.
    token_owner: FileTokenOwner,
    /// Transitional parser compatibility vector. This is materialized only at explicit AST
    /// expression/parser handoff boundaries; retained declaration/signature shells keep ranges.
    pub tokens: Vec<Token>,
    /// File-owned authored path trees referenced by `TokenKind::Path` handles.
    ///
    /// This lifecycle adapter remains mutable while header preparation remaps and rebinds the
    /// source. Frozen source-token consumers receive the same allocation through `SourceTokens`.
    pub path_syntax: FilePathSyntax,
    /// Numeric side-store handle for each token position, when that token is numeric.
    ///
    /// The records themselves live in the canonical `SourceTokens` owner or, for adapters,
    /// in the adapter numeric store; this positional lane is retained only for compatibility with
    /// generic parser capture code.
    #[cfg(test)]
    pub(crate) numeric_literal_ids: Vec<Option<NumericLiteralId>>,
    /// Complete logical identity of the owning source file in the active path table.
    pub src_path: PathId,
    /// Required owning source identity for every token stream, including materialised generics.
    pub file_id: SourceId,
    /// Canonical filesystem source path for IO/path-resolution-only logic.
    ///
    /// This is adapter metadata, not part of immutable source token storage. The preparation
    /// output remains the long-lived filesystem identity owner.
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
    #[cfg(test)]
    pub fn new_with_identity(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
    ) -> FileTokens {
        Self::with_canonical_path_syntax(
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
        Self::with_canonical_path_syntax_and_numeric_store(
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
    #[cfg(test)]
    pub(crate) fn new_frozen(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
    ) -> FileTokens {
        let mut stream = Self::with_canonical_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::shared(path_syntax),
        );
        stream.freeze_numeric_literals();
        stream
    }

    /// Construct a retained token stream that will receive its table from the completed
    /// prepared-file owner.
    pub fn new_deferred_with_identity(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
    ) -> FileTokens {
        Self::with_adapter_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            FilePathSyntax::Deferred,
        )
    }

    #[cfg(test)]
    fn with_canonical_path_syntax(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
    ) -> FileTokens {
        let (numeric_literals, numeric_literal_ids) = numeric_store_from_tokens(file_id, &tokens);
        Self::with_canonical_path_syntax_and_numeric_store(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            path_syntax,
            numeric_literals,
            numeric_literal_ids,
        )
    }

    fn with_adapter_path_syntax(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
    ) -> FileTokens {
        let (numeric_literals, numeric_literal_ids) = numeric_store_from_tokens(file_id, &tokens);
        Self::with_adapter_path_syntax_and_numeric_store(
            AdapterIdentity {
                src_path,
                file_id,
                canonical_os_path,
            },
            tokens,
            path_syntax,
            numeric_literals,
            numeric_literal_ids,
            FileTokenAdapterMetadata::Unbounded,
        )
    }

    fn with_adapter_path_syntax_and_metadata(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
        metadata: FileTokenAdapterMetadata,
    ) -> FileTokens {
        let (numeric_literals, numeric_literal_ids) = numeric_store_from_tokens(file_id, &tokens);
        Self::with_adapter_path_syntax_and_numeric_store(
            AdapterIdentity {
                src_path,
                file_id,
                canonical_os_path,
            },
            tokens,
            path_syntax,
            numeric_literals,
            numeric_literal_ids,
            metadata,
        )
    }

    fn with_canonical_path_syntax_and_numeric_store(
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
        let (source_tokens, staged_ids) = SourceTokens::from_legacy_tokens(
            file_id,
            &tokens,
            numeric_literals,
            Some(&numeric_literal_ids),
            path_syntax.shared_table_for_source_tokens(),
            TokenStats::default(),
        )
        .expect("legacy token adapter must produce valid source-token shapes");
        debug_assert_eq!(
            staged_ids, numeric_literal_ids,
            "source-token and parser numeric handle lanes must stay aligned"
        );
        // The canonical owner counted from its packed shapes during construction, so the
        // shell shares that exact snapshot instead of re-running a default-then-rewrite pass.
        let token_stats = source_tokens.token_stats();
        #[cfg(not(test))]
        drop(numeric_literal_ids);
        FileTokens {
            length: tokens.len(),
            token_owner: FileTokenOwner::Canonical(Arc::new(source_tokens)),
            path_syntax,
            #[cfg(test)]
            numeric_literal_ids,
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            token_stats,
            index: 0,
        }
    }

    fn with_adapter_path_syntax_and_numeric_store(
        identity: AdapterIdentity,
        tokens: Vec<Token>,
        path_syntax: FilePathSyntax,
        numeric_literals: NumericLiteralStore,
        numeric_literal_ids: Vec<Option<NumericLiteralId>>,
        metadata: FileTokenAdapterMetadata,
    ) -> FileTokens {
        let AdapterIdentity {
            src_path,
            file_id,
            canonical_os_path,
        } = identity;
        debug_assert_eq!(
            tokens.len(),
            numeric_literal_ids.len(),
            "numeric side-store handles must align with token positions"
        );
        validate_adapter_numeric_lane(file_id, &tokens, &numeric_literals, &numeric_literal_ids);
        if let FilePathSyntax::Shared(table) = &path_syntax {
            table
                .validate_file_owned_locations(file_id)
                .expect("parser adapter path table must match its source identity");
            table
                .validate_file_tokens(&tokens, file_id, "parser adapter stream")
                .expect("parser adapter path handles must match its shared table");
        }
        #[cfg(not(test))]
        drop(numeric_literal_ids);
        FileTokens {
            length: tokens.len(),
            token_owner: FileTokenOwner::Adapter {
                numeric_literals,
                metadata,
            },
            path_syntax,
            #[cfg(test)]
            numeric_literal_ids,
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            token_stats: TokenStats::default(),
            index: 0,
        }
    }
    /// Build a bounded parser substream over a token slice.
    ///
    /// Header parsing defers the path-table attachment until the prepared-file owner freezes.
    /// Later AST substreams clone only the immutable table handle. This adapter owns no canonical
    /// `SourceTokens`; it keeps only the parser `Token` vector, numeric side store and lifecycle
    /// shell for its bounded lifetime.
    #[cfg(test)]
    pub fn new_substream(
        source: &FileTokens,
        src_path: PathId,
        file_id: SourceId,
        tokens: Vec<Token>,
    ) -> FileTokens {
        Self::with_adapter_path_syntax(
            src_path,
            file_id,
            source.canonical_os_path.clone(),
            tokens,
            source.path_syntax.permanent_substream(),
        )
    }

    /// Build a downstream parser stream over already-frozen file syntax.
    ///
    /// AST expression parsers use this compatibility-only constructor for copied token vectors.
    /// It deliberately retains no canonical provenance; `canonical_cursor_from_current` returns
    /// an honest `CompilerError` for such a stream instead of fabricating a duplicate owner.
    /// Bounded declaration adapters must use `new_bounded_substream`,
    /// `new_bounded_sequence_substream`, or `new_remapped_bounded_adapter`.
    pub fn new_from_slice(
        src_path: PathId,
        file_id: SourceId,
        canonical_os_path: Option<PathBuf>,
        tokens: Vec<Token>,
        source_path_syntax: &FilePathSyntax,
    ) -> Result<FileTokens, CompilerError> {
        let mut stream = Self::with_adapter_path_syntax(
            src_path,
            file_id,
            canonical_os_path,
            tokens,
            source_path_syntax.frozen_substream()?,
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }
    fn validate_remapped_token_pair(
        canonical: TokenRef<'_>,
        remapped: &Token,
    ) -> Result<(), CompilerError> {
        if canonical.span() != remapped.span || canonical.tag() != remapped.kind.token_tag() {
            return Err(CompilerError::compiler_error(
                "remapped compatibility token no longer matches canonical span or tag",
            ));
        }
        Ok(())
    }
    /// Build a frozen bounded parser adapter from a transiently remapped token slice.
    ///
    /// Remapping changes only the compatibility payload IDs; canonical spans/tags remain owned by
    /// `source`. The adapter therefore retains the donor `Arc<SourceTokens>` and its checked range
    /// or sequence metadata rather than constructing a second canonical source owner.
    pub(crate) fn new_remapped_bounded_adapter(
        source: &FileTokens,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
        declaration_path: PathId,
        tokens: Vec<Token>,
        path_syntax: PathSyntaxTable,
    ) -> Result<FileTokens, CompilerError> {
        if token_range.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "remapped bounded adapter range does not match its source identity",
            ));
        }
        let source_tokens = source.token_owner.canonical_arc()?;
        if source_tokens.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "remapped bounded adapter canonical owner does not match its source identity",
            ));
        }

        let metadata = if let Some(sequence) = token_sequence {
            let view = source_tokens.token_sequence(sequence).map_err(|error| {
                CompilerError::compiler_error(format!(
                    "remapped bounded adapter sequence provenance is invalid: {error:?}"
                ))
            })?;
            if tokens.len() != view.len() {
                return Err(CompilerError::compiler_error(
                    "remapped bounded adapter token count does not match its sequence",
                ));
            }
            let mut cursor = view.cursor().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "remapped bounded adapter sequence cursor is invalid: {error:?}"
                ))
            })?;
            for token in &tokens {
                let canonical = cursor.advance().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "remapped bounded adapter sequence ended before its compatibility lane",
                    )
                })?;
                Self::validate_remapped_token_pair(canonical, token)?;
            }
            FileTokenAdapterMetadata::Sequence {
                source_tokens: Arc::clone(&source_tokens),
                sequence,
            }
        } else {
            let mut cursor = source_tokens.cursor(token_range).map_err(|error| {
                CompilerError::compiler_error(format!(
                    "remapped bounded adapter range provenance is invalid: {error:?}"
                ))
            })?;
            if tokens.len() != token_range.len() as usize {
                return Err(CompilerError::compiler_error(
                    "remapped bounded adapter token count does not match its range",
                ));
            }
            for token in &tokens {
                let canonical = cursor.advance().ok_or_else(|| {
                    CompilerError::compiler_error(
                        "remapped bounded adapter range ended before its compatibility lane",
                    )
                })?;
                Self::validate_remapped_token_pair(canonical, token)?;
            }
            FileTokenAdapterMetadata::Contiguous {
                source_tokens: Arc::clone(&source_tokens),
                range: token_range,
                synthetic_trailing_eof: false,
            }
        };

        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            source.file_id,
            source.canonical_os_path.clone(),
            tokens,
            FilePathSyntax::shared(path_syntax),
            metadata,
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }

    /// Build a bounded parser adapter from one contiguous canonical range.
    ///
    /// The adapter keeps a checked range and an `Arc` to the canonical source solely for the
    /// transient declaration-parser handoff. Its legacy vector remains available to expression
    /// parser code, but no `SourceTokens` rows are copied into the adapter.
    pub(crate) fn new_bounded_substream(
        source: &FileTokens,
        range: TokenRange,
        declaration_path: PathId,
    ) -> Result<FileTokens, CompilerError> {
        if range.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "retained token range does not match its source stream identity",
            ));
        }
        let source_tokens = source.token_owner.canonical_arc()?;
        if source_tokens.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "bounded token range source owner does not match its file stream identity",
            ));
        }
        let tokens = source.materialize_token_range(range)?;
        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            source.file_id,
            source.canonical_os_path.clone(),
            tokens,
            source.path_syntax.frozen_substream()?,
            FileTokenAdapterMetadata::Contiguous {
                source_tokens,
                range,
                synthetic_trailing_eof: false,
            },
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }
    /// Build a bounded expression-parser adapter with an ephemeral EOF sentinel.
    ///
    /// The authored expression remains backed by `range` in the canonical owner. The trailing
    /// EOF is compatibility-only parser data and is excluded from canonical cursor positions.
    pub(crate) fn new_bounded_expression_substream(
        source: &FileTokens,
        range: TokenRange,
        declaration_path: PathId,
        eof_span: LocalSpan,
    ) -> Result<FileTokens, CompilerError> {
        if range.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "retained expression range does not match its source stream identity",
            ));
        }
        let source_tokens = source.token_owner.canonical_arc()?;
        if source_tokens.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "bounded expression range source owner does not match its file stream identity",
            ));
        }
        let mut tokens = source.materialize_token_range(range)?;
        tokens.push(Token::new(TokenKind::Eof, eof_span));
        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            source.file_id,
            source.canonical_os_path.clone(),
            tokens,
            source.path_syntax.frozen_substream()?,
            FileTokenAdapterMetadata::Contiguous {
                source_tokens,
                range,
                synthetic_trailing_eof: true,
            },
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }
    /// Build a bounded expression adapter directly from an already-borrowed canonical owner.
    ///
    /// This is the handoff used by `AstCursor::new_bounded_expression_substream` when the cursor
    /// was created from a `FileTokens` owner. The canonical `SourceTokens` allocation is shared;
    /// only the explicit parser compatibility vector is materialized.
    pub(crate) fn new_bounded_expression_substream_from_canonical(
        source_tokens: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        range: TokenRange,
        declaration_path: PathId,
        eof_span: LocalSpan,
    ) -> Result<FileTokens, CompilerError> {
        let file_id = source_tokens.source();
        source_tokens.validate_range(range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "retained expression range does not match its source identity: {error:?}"
            ))
        })?;
        let mut tokens = source_tokens.materialize_range(range)?;
        tokens.push(Token::new(TokenKind::Eof, eof_span));
        let path_syntax = FilePathSyntax::Shared(source_tokens.path_syntax_arc()?);
        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            file_id,
            canonical_os_path,
            tokens,
            path_syntax,
            FileTokenAdapterMetadata::Contiguous {
                source_tokens,
                range,
                synthetic_trailing_eof: true,
            },
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }
    /// Build a bounded parser adapter directly from one canonical source owner.
    ///
    /// The canonical `SourceTokens` allocation is shared; only the explicit parser
    /// compatibility vector is materialized. This is the source-body handoff used once
    /// retained generic syntax owns `Arc<SourceTokens>` instead of a `FileTokens` shell.
    pub(crate) fn new_bounded_substream_from_canonical(
        source_tokens: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        range: TokenRange,
        declaration_path: PathId,
    ) -> Result<FileTokens, CompilerError> {
        let file_id = source_tokens.source();
        source_tokens.validate_range(range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "retained token range does not match its source identity: {error:?}"
            ))
        })?;
        let tokens = source_tokens.materialize_range(range)?;
        let path_syntax = FilePathSyntax::Shared(source_tokens.path_syntax_arc()?);
        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            file_id,
            canonical_os_path,
            tokens,
            path_syntax,
            FileTokenAdapterMetadata::Contiguous {
                source_tokens,
                range,
                synthetic_trailing_eof: false,
            },
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }

    /// Build a bounded segmented-sequence adapter directly from one canonical source owner.
    ///
    /// The canonical `SourceTokens` allocation is shared; only the explicit parser
    /// compatibility vector is materialized. Sequence provenance keeps donor gaps explicit
    /// while no second canonical owner is constructed.
    pub(crate) fn new_bounded_sequence_substream_from_canonical(
        source_tokens: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        sequence: TokenSequenceId,
        declaration_path: PathId,
    ) -> Result<FileTokens, CompilerError> {
        let view = source_tokens.token_sequence(sequence).map_err(|error| {
            CompilerError::compiler_error(format!(
                "bounded token sequence provenance is invalid: {error:?}"
            ))
        })?;
        let mut cursor = view.cursor().map_err(|error| {
            CompilerError::compiler_error(format!(
                "token sequence cursor construction failed: {error:?}"
            ))
        })?;
        let mut tokens = Vec::with_capacity(view.len());
        while let Some(token_ref) = cursor.advance() {
            let is_eof = token_ref.is_eof();
            let kind = token_ref.to_token_kind().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "canonical token payload could not be materialized: {error:?}"
                ))
            })?;
            tokens.push(Token::new(kind, token_ref.span()));
            if is_eof {
                break;
            }
        }
        let file_id = source_tokens.source();
        let path_syntax = FilePathSyntax::Shared(source_tokens.path_syntax_arc()?);
        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            file_id,
            canonical_os_path,
            tokens,
            path_syntax,
            FileTokenAdapterMetadata::Sequence {
                source_tokens,
                sequence,
            },
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }
    /// Wrap one canonical owner in a `FileTokens` shell that retains one checked body range.
    ///
    /// Materialised generic bodies must keep an `Arc<FileTokens>` donor shell while sharing one
    /// canonical `SourceTokens` allocation. This builds that compatibility shell directly from the
    /// shared owner plus its filesystem identity so Source-origin frozen syntax can materialise
    /// without constructing a second canonical store. The retained range (or sequence) is the
    /// same checked provenance that `GenericFunctionBody::materialised` validates, so the shell
    /// stays usable through the existing `FileTokens`-shell validator and bounded adapters.
    pub(crate) fn canonical_shell_from_canonical(
        source_tokens: Arc<SourceTokens>,
        canonical_os_path: Option<PathBuf>,
        src_path: PathId,
        token_range: TokenRange,
        token_sequence: Option<TokenSequenceId>,
    ) -> Result<FileTokens, CompilerError> {
        match token_sequence {
            Some(sequence) => Self::new_bounded_sequence_substream_from_canonical(
                source_tokens,
                canonical_os_path,
                sequence,
                src_path,
            ),
            None => Self::new_bounded_substream_from_canonical(
                source_tokens,
                canonical_os_path,
                token_range,
                src_path,
            ),
        }
    }

    /// Build the bounded parser adapter for one source-owned segmented sequence.
    pub(crate) fn new_bounded_sequence_substream(
        source: &FileTokens,
        sequence: TokenSequenceId,
        declaration_path: PathId,
    ) -> Result<FileTokens, CompilerError> {
        let source_tokens = source.token_owner.canonical_arc()?;
        if source_tokens.source() != source.file_id {
            return Err(CompilerError::compiler_error(
                "bounded token sequence source owner does not match its file stream identity",
            ));
        }
        let tokens = source.materialize_token_sequence(sequence)?;
        source_tokens.token_sequence(sequence).map_err(|error| {
            CompilerError::compiler_error(format!(
                "bounded token sequence provenance is invalid: {error:?}"
            ))
        })?;
        let mut stream = Self::with_adapter_path_syntax_and_metadata(
            declaration_path,
            source.file_id,
            source.canonical_os_path.clone(),
            tokens,
            source.path_syntax.frozen_substream()?,
            FileTokenAdapterMetadata::Sequence {
                source_tokens,
                sequence,
            },
        );
        stream.freeze_numeric_literals();
        Ok(stream)
    }

    /// Move this source stream into the prepared-source owner.
    ///
    /// The replacement is an empty adapter used only to leave the caller's mutable slot in a
    /// valid state. The moved stream retains the one canonical `SourceTokens` owner.
    pub(crate) fn take_for_prepared_source(&mut self) -> FileTokens {
        let replacement = Self::new_deferred_with_identity(
            self.src_path,
            self.file_id,
            self.canonical_os_path.clone(),
            Vec::new(),
        );
        std::mem::replace(self, replacement)
    }

    pub(crate) fn try_register_token_sequence(
        &mut self,
        ranges: &[TokenRange],
    ) -> Result<TokenSequenceId, TokenSequenceError> {
        self.token_owner
            .as_canonical_mut()
            .map_err(|_| TokenSequenceError::NoCanonicalOwner)?
            .try_register_token_sequence(ranges)
    }

    pub(crate) fn register_token_sequence(
        &mut self,
        ranges: &[TokenRange],
    ) -> Result<TokenSequenceId, CompilerError> {
        self.try_register_token_sequence(ranges).map_err(|error| {
            CompilerError::compiler_error(format!(
                "source token sequence registration failed: {error:?}"
            ))
        })
    }

    ///
    /// Ordinary parser/header adapters own no `SourceTokens`; their honest boundary is this
    /// error rather than a duplicate source-qualified owner.
    pub fn source_tokens(&self) -> Result<&SourceTokens, CompilerError> {
        self.token_owner.as_canonical()
    }
    /// Borrow the canonical source store through a bounded adapter's retained provenance.
    ///
    /// Unlike `source_tokens`, this includes adapters that share an existing canonical owner.
    /// Unbounded compatibility vectors still fail rather than fabricating source identity.
    pub(crate) fn canonical_source_tokens(&self) -> Result<&SourceTokens, CompilerError> {
        self.token_owner.canonical_source_tokens()
    }
    /// Clone the canonical source owner retained by this stream.
    ///
    /// AST cursors keep this handle when they are created from a `FileTokens` stream so a later
    /// bounded expression handoff can share the same source allocation without rebuilding it.
    pub(crate) fn canonical_source_tokens_arc(&self) -> Result<Arc<SourceTokens>, CompilerError> {
        self.token_owner.canonical_arc()
    }
    /// Translate a canonical source-token index into this stream's compatibility-vector index.
    ///
    /// Canonical and unbounded streams retain source-indexed vectors, so they use an identity
    /// mapping. Contiguous adapters subtract their retained range start; segmented adapters walk
    /// the checked sequence and omit source gaps. Synthetic trailing EOF entries have no canonical
    /// source index and therefore are not mapped.
    pub(crate) fn compatibility_index_for_source_index(
        &self,
        source_index: usize,
    ) -> Option<usize> {
        match &self.token_owner {
            FileTokenOwner::Canonical(_) => Some(source_index),
            FileTokenOwner::Adapter { metadata, .. } => match metadata {
                FileTokenAdapterMetadata::Unbounded => Some(source_index),
                FileTokenAdapterMetadata::Contiguous { range, .. } => {
                    let start = range.start().index();
                    let end = range.end().index();
                    (start..end)
                        .contains(&source_index)
                        .then(|| source_index.checked_sub(start))
                        .flatten()
                }
                FileTokenAdapterMetadata::Sequence {
                    source_tokens,
                    sequence,
                } => {
                    let view = source_tokens.token_sequence(*sequence).ok()?;
                    let mut compatibility_index = 0usize;
                    for segment in view.ranges() {
                        let start = segment.start().index();
                        let end = segment.end().index();
                        if (start..end).contains(&source_index) {
                            return compatibility_index.checked_add(source_index - start);
                        }
                        compatibility_index =
                            compatibility_index.checked_add(segment.len() as usize)?;
                    }
                    None
                }
            },
        }
    }
    /// Create a checked canonical cursor at this stream's compatibility-vector position.
    ///
    /// Canonical streams use their full source range. Bounded adapters use the retained range or
    /// sequence metadata and an `Arc` clone of that same owner. Compatibility-only vectors made
    /// by [`Self::new_from_slice`] deliberately return an error instead of fabricating a source
    /// owner.
    pub fn canonical_cursor_from_current(&self) -> Result<TokenCursor<'_>, CompilerError> {
        if self.length != self.tokens.len() || self.index > self.length {
            return Err(CompilerError::compiler_error(
                "token adapter compatibility position is outside its vector bounds",
            ));
        }
        match &self.token_owner {
            FileTokenOwner::Canonical(source_tokens) => {
                if source_tokens.source() != self.file_id {
                    return Err(CompilerError::compiler_error(
                        "canonical source token owner does not match its file stream identity",
                    ));
                }
                let range = source_tokens.full_range().map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical source token range could not be constructed: {error:?}"
                    ))
                })?;
                let position = TokenIndex::try_from_index(self.index).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "canonical compatibility position exceeded its checked index domain",
                    )
                })?;
                TokenCursor::from_range_position(source_tokens, range, position).map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical cursor handoff position was out of bounds: {error:?}"
                    ))
                })
            }
            FileTokenOwner::Adapter { metadata, .. } => match metadata {
                FileTokenAdapterMetadata::Unbounded => Err(CompilerError::compiler_error(
                    "token adapter has no canonical source-token provenance",
                )),
                FileTokenAdapterMetadata::Contiguous {
                    source_tokens,
                    range,
                    synthetic_trailing_eof,
                } => {
                    if source_tokens.source() != self.file_id || range.source() != self.file_id {
                        return Err(CompilerError::compiler_error(
                            "bounded token adapter provenance does not match its file identity",
                        ));
                    }
                    let expected_length =
                        range.len() as usize + if *synthetic_trailing_eof { 1 } else { 0 };
                    if expected_length != self.length {
                        return Err(CompilerError::compiler_error(
                            "bounded token adapter length does not match its canonical range",
                        ));
                    }
                    let offset = TokenIndex::try_from_index(self.index).ok_or_else(|| {
                        CompilerError::compiler_error(
                            "bounded compatibility position exceeded its checked index domain",
                        )
                    })?;
                    let position =
                        TokenIndex(range.start().raw().checked_add(offset.raw()).ok_or_else(
                            || {
                                CompilerError::compiler_error(
                                    "bounded canonical cursor position overflowed its index domain",
                                )
                            },
                        )?);
                    TokenCursor::from_range_position(source_tokens, *range, position).map_err(
                        |error| {
                            CompilerError::compiler_error(format!(
                                "bounded canonical cursor handoff failed: {error:?}"
                            ))
                        },
                    )
                }
                FileTokenAdapterMetadata::Sequence {
                    source_tokens,
                    sequence,
                } => {
                    if source_tokens.source() != self.file_id {
                        return Err(CompilerError::compiler_error(
                            "segmented token adapter provenance does not match its file identity",
                        ));
                    }
                    let view = source_tokens.token_sequence(*sequence).map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "segmented token adapter provenance is invalid: {error:?}"
                        ))
                    })?;
                    if view.len() != self.length {
                        return Err(CompilerError::compiler_error(
                            "segmented token adapter length does not match its canonical sequence",
                        ));
                    }
                    TokenCursor::from_sequence_position(view, self.index, self.length).map_err(
                        |error| {
                            CompilerError::compiler_error(format!(
                                "segmented canonical cursor handoff failed: {error:?}"
                            ))
                        },
                    )
                }
            },
        }
    }

    /// Compute the legacy compatibility index after a transient canonical parser handoff.
    ///
    /// The cursor must have been created for this exact owner and range/sequence metadata. A
    /// foreign or differently bounded cursor is rejected rather than silently producing an
    /// index for an unrelated source position.
    pub fn compatibility_index_for_cursor(
        &self,
        cursor: TokenCursor<'_>,
    ) -> Result<usize, CompilerError> {
        if self.length != self.tokens.len() {
            return Err(CompilerError::compiler_error(
                "token adapter compatibility vector length is inconsistent",
            ));
        }
        let next_index = match &self.token_owner {
            FileTokenOwner::Canonical(source_tokens) => {
                if !std::ptr::eq(cursor.tokens, source_tokens.as_ref())
                    || cursor.tokens.source() != self.file_id
                {
                    return Err(CompilerError::compiler_error(
                        "canonical cursor belongs to a different source-token owner",
                    ));
                }
                let range = source_tokens.full_range().map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "canonical source token range could not be constructed: {error:?}"
                    ))
                })?;
                if !matches!(cursor.bounds, TokenCursorBounds::Contiguous(actual) if actual == range)
                {
                    return Err(CompilerError::compiler_error(
                        "canonical cursor bounds do not match the owning file stream",
                    ));
                }
                cursor.position().index()
            }
            FileTokenOwner::Adapter { metadata, .. } => match metadata {
                FileTokenAdapterMetadata::Unbounded => {
                    return Err(CompilerError::compiler_error(
                        "token adapter has no canonical source-token provenance",
                    ));
                }
                FileTokenAdapterMetadata::Contiguous {
                    source_tokens,
                    range,
                    synthetic_trailing_eof,
                } => {
                    if !std::ptr::eq(cursor.tokens, source_tokens.as_ref())
                        || cursor.tokens.source() != self.file_id
                    {
                        return Err(CompilerError::compiler_error(
                            "bounded cursor belongs to a different source-token owner",
                        ));
                    }
                    if !matches!(cursor.bounds, TokenCursorBounds::Contiguous(actual) if actual == *range)
                    {
                        return Err(CompilerError::compiler_error(
                            "bounded cursor range does not match the adapter provenance",
                        ));
                    }
                    if cursor.position() < range.start() || cursor.position() > range.end() {
                        return Err(CompilerError::compiler_error(
                            "bounded cursor position is outside the adapter range",
                        ));
                    }
                    let compatibility_index = cursor
                        .position()
                        .index()
                        .checked_sub(range.start().index())
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "bounded cursor position underflowed its adapter index",
                            )
                        })?;
                    if *synthetic_trailing_eof && compatibility_index == range.len() as usize {
                        self.length.saturating_sub(1)
                    } else {
                        compatibility_index
                    }
                }
                FileTokenAdapterMetadata::Sequence {
                    source_tokens,
                    sequence,
                } => {
                    if !std::ptr::eq(cursor.tokens, source_tokens.as_ref())
                        || cursor.tokens.source() != self.file_id
                    {
                        return Err(CompilerError::compiler_error(
                            "segmented cursor belongs to a different source-token owner",
                        ));
                    }
                    let TokenCursorBounds::Segmented(view) = cursor.bounds else {
                        return Err(CompilerError::compiler_error(
                            "segmented adapter requires a segmented canonical cursor",
                        ));
                    };
                    if !std::ptr::eq(view.tokens, source_tokens.as_ref()) || view.id() != *sequence
                    {
                        return Err(CompilerError::compiler_error(
                            "segmented cursor sequence does not match the adapter provenance",
                        ));
                    }
                    cursor.compatibility_position().map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "segmented cursor position could not be synchronised: {error:?}"
                        ))
                    })?
                }
            },
        };
        if next_index > self.length {
            return Err(CompilerError::compiler_error(
                "canonical cursor consumed beyond its compatibility adapter",
            ));
        }
        Ok(next_index)
    }

    pub(crate) fn path_syntax_table(&self) -> Result<&PathSyntaxTable, CompilerError> {
        self.path_syntax.table()
    }

    /// Materialize one checked token range while preserving an adapter's transient payload lane.
    ///
    /// Canonical streams decode from the immutable source arrays. Bounded adapters may carry
    /// rebased string/path IDs in their compatibility vector, so nested expression handoffs must
    /// slice that vector instead of decoding the donor canonical owner again.
    pub(crate) fn materialize_token_range(
        &self,
        range: TokenRange,
    ) -> Result<Vec<Token>, CompilerError> {
        match &self.token_owner {
            FileTokenOwner::Canonical(_) => self.materialize_canonical_token_range(range),
            FileTokenOwner::Adapter { .. } => self.materialize_adapter_token_range(range),
        }
    }

    fn materialize_canonical_token_range(
        &self,
        range: TokenRange,
    ) -> Result<Vec<Token>, CompilerError> {
        let canonical = self.canonical_source_tokens()?;
        if canonical.source() != self.file_id {
            return Err(CompilerError::compiler_error(
                "canonical source token owner does not match its file stream identity",
            ));
        }
        let cursor = canonical.cursor(range).map_err(|error| {
            CompilerError::compiler_error(format!("token range materialization failed: {error:?}"))
        })?;
        self.materialize_cursor(cursor)
    }

    fn materialize_adapter_token_range(
        &self,
        range: TokenRange,
    ) -> Result<Vec<Token>, CompilerError> {
        if range.source() != self.file_id {
            return Err(CompilerError::compiler_error(
                "adapter token range does not match its file stream identity",
            ));
        }
        let FileTokenOwner::Adapter { metadata, .. } = &self.token_owner else {
            unreachable!("canonical owners use materialize_canonical_token_range");
        };
        match metadata {
            FileTokenAdapterMetadata::Unbounded => Err(CompilerError::compiler_error(
                "unbounded token adapter has no canonical range provenance",
            )),
            FileTokenAdapterMetadata::Contiguous {
                range: owner_range, ..
            } => {
                if range.start() < owner_range.start() || range.end() > owner_range.end() {
                    return Err(CompilerError::compiler_error(
                        "adapter token range exceeds its contiguous provenance",
                    ));
                }
                let start = range
                    .start()
                    .index()
                    .checked_sub(owner_range.start().index())
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "adapter token range start underflowed its provenance",
                        )
                    })?;
                let end = range
                    .end()
                    .index()
                    .checked_sub(owner_range.start().index())
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "adapter token range end underflowed its provenance",
                        )
                    })?;
                self.tokens
                    .get(start..end)
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "adapter token range exceeded its compatibility vector",
                        )
                    })
            }
            FileTokenAdapterMetadata::Sequence {
                source_tokens,
                sequence,
            } => {
                let view = source_tokens.token_sequence(*sequence).map_err(|error| {
                    CompilerError::compiler_error(format!(
                        "adapter token sequence provenance is invalid: {error:?}"
                    ))
                })?;
                if self.tokens.len() != view.len() {
                    return Err(CompilerError::compiler_error(
                        "adapter token sequence length does not match its compatibility vector",
                    ));
                }

                let requested_start = range.start().index();
                let requested_end = range.end().index();
                let mut compatibility_start = 0usize;
                let mut materialized = Vec::with_capacity(range.len() as usize);
                for segment in view.ranges() {
                    let segment_start = segment.start().index();
                    let segment_end = segment.end().index();
                    let overlap_start = requested_start.max(segment_start);
                    let overlap_end = requested_end.min(segment_end);
                    if overlap_start < overlap_end {
                        let start = compatibility_start
                            .checked_add(overlap_start - segment_start)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "adapter token sequence start overflowed its compatibility vector",
                                )
                            })?;
                        let end = compatibility_start
                            .checked_add(overlap_end - segment_start)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(
                                    "adapter token sequence end overflowed its compatibility vector",
                                )
                            })?;
                        let tokens = self.tokens.get(start..end).ok_or_else(|| {
                            CompilerError::compiler_error(
                                "adapter token sequence range exceeded its compatibility vector",
                            )
                        })?;
                        materialized.extend_from_slice(tokens);
                    }
                    compatibility_start = compatibility_start
                        .checked_add(segment.len() as usize)
                        .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "adapter token sequence length overflowed its compatibility vector",
                        )
                    })?;
                }
                if materialized.len() != range.len() as usize {
                    return Err(CompilerError::compiler_error(
                        "adapter token range crosses an omitted segmented-source gap",
                    ));
                }
                Ok(materialized)
            }
        }
    }

    /// Materialize one checked segmented sequence through the canonical cursor.
    pub(crate) fn materialize_token_sequence(
        &self,
        id: TokenSequenceId,
    ) -> Result<Vec<Token>, CompilerError> {
        let canonical = self.canonical_source_tokens()?;
        if canonical.source() != self.file_id {
            return Err(CompilerError::compiler_error(
                "canonical source token owner does not match its file stream identity",
            ));
        }
        let view = canonical.token_sequence(id).map_err(|error| {
            CompilerError::compiler_error(format!(
                "token sequence view construction failed: {error:?}"
            ))
        })?;
        let cursor = view.cursor().map_err(|error| {
            CompilerError::compiler_error(format!(
                "token sequence cursor construction failed: {error:?}"
            ))
        })?;
        self.materialize_cursor(cursor)
    }

    fn materialize_cursor(&self, mut cursor: TokenCursor<'_>) -> Result<Vec<Token>, CompilerError> {
        let mut tokens = Vec::with_capacity(cursor.range().len() as usize);
        while let Some(token_ref) = cursor.advance() {
            let is_eof = token_ref.is_eof();
            let kind = token_ref.to_token_kind().map_err(|error| {
                CompilerError::compiler_error(format!(
                    "canonical token payload could not be materialized: {error:?}"
                ))
            })?;
            tokens.push(Token::new(kind, token_ref.span()));
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    /// Whether this stream is the sole canonical `SourceTokens` owner for its construction.
    pub(crate) const fn has_canonical_source_tokens(&self) -> bool {
        self.token_owner.is_canonical()
    }

    /// Return the numeric cold store for this stream.
    ///
    /// Canonical owners expose the SoA cold store; adapters expose their numeric side store with
    /// the same source/handle lifecycle.
    #[cfg(test)]
    pub(crate) fn numeric_literal_store(&self) -> &NumericLiteralStore {
        self.token_owner.numeric_literal_store()
    }

    #[cfg(test)]
    pub(crate) fn numeric_literal_id_at(&self, token_index: usize) -> Option<NumericLiteralId> {
        self.numeric_literal_ids.get(token_index).copied().flatten()
    }

    /// Move the sole mutable path-table owner into a prepared-file output.
    pub(crate) fn take_preparing_path_syntax(
        &mut self,
    ) -> Result<Arc<PathSyntaxTable>, CompilerError> {
        self.path_syntax.take_preparing_table()
    }

    /// Commit a table attachment after the whole-file preflight passed for every header.
    ///
    /// Canonical owners also receive the same frozen allocation in their `SourceTokens` slot so
    /// cursor path views resolve after ordinary publication. Adapters share only the lifecycle
    /// shell handle because they own no SoA table slot.
    #[cfg(test)]
    pub(crate) fn attach_preflighted_shared_path_syntax(
        &mut self,
        path_syntax: Arc<PathSyntaxTable>,
    ) {
        self.token_owner
            .attach_owner_shared_path_syntax(Arc::clone(&path_syntax));
        self.path_syntax.attach_preflighted_shared(path_syntax);
    }

    /// Freeze the numeric store at publication.
    ///
    /// The caller must complete all construction-time string remaps first. Generic
    /// materialisation's remapped clone finishes frozen through its own checked owner
    /// boundary and never calls this on an already-frozen store.
    pub(crate) fn freeze_numeric_literals(&mut self) {
        self.token_owner.freeze_owner_numeric_literals();
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
                self.token_owner
                    .attach_owner_shared_path_syntax(Arc::clone(&path_syntax));
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
    /// Return the exact global span of the current token.
    pub fn current_span(&self) -> SourceSpan {
        SourceSpan::new(self.file_id, self.tokens[self.index].span)
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

    #[cfg(test)]
    /// Remap a token stream while it still owns its mutable path table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if self.numeric_literal_store().is_frozen() {
            panic!("numeric literal remapping was requested after the source publication freeze");
        }
        self.token_owner.remap_owner_string_ids(remap);
        self.remap_token_payload_string_ids(remap);
    }

    #[cfg(test)]
    /// Remap this stream's complete-path identity after its path fork merges.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.src_path = remap.get(self.src_path);
    }

    #[cfg(test)]
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
        self.path_syntax.preparing_table_mut()?;
        if self.numeric_literal_store().is_frozen() {
            return Err(CompilerError::compiler_error(
                "numeric literal remapping was requested after the source publication freeze",
            ));
        }
        self.token_owner.remap_owner_string_ids(remap);
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
        self.token_owner.rebind_owner_identity(file_id);
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
/// Validate one adapter numeric lane without building a second canonical owner.
///
/// Adapters reuse the ordinary numeric handle/source checks so parser streams keep working,
/// but they never allocate `SourceTokens` shapes, spans or cold-store SoA arrays.
fn validate_adapter_numeric_lane(
    source: SourceId,
    tokens: &[Token],
    numeric_literals: &NumericLiteralStore,
    numeric_literal_ids: &[Option<NumericLiteralId>],
) {
    debug_assert_eq!(
        tokens.len(),
        numeric_literal_ids.len(),
        "parser adapter numeric handles must align with token positions"
    );
    if numeric_literals.owner_source().is_none() && !numeric_literals.is_empty() {
        panic!("parser adapter numeric store has records but no source identity");
    }
    for (index, token) in tokens.iter().enumerate() {
        let numeric_id = numeric_literal_ids[index];
        let is_numeric = matches!(token.kind, TokenKind::NumericLiteral(_));
        match (is_numeric, numeric_id) {
            (true, Some(id)) => {
                numeric_literals
                    .try_get_for_source(id, source)
                    .expect("parser adapter numeric handle must address its numeric side store");
            }
            (true, None) => panic!(
                "parser adapter numeric token at index {index} is missing its side-store handle"
            ),
            (false, Some(_)) => {
                panic!("parser adapter non-numeric token at index {index} carries a numeric handle")
            }
            (false, None) => {}
        }
    }
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

    /// Build the compact shape for a transient `TokenKind` adapter.
    #[cfg(test)]
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

impl Token {
    #[cfg(test)]
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
    #[cfg(test)]
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

    #[cfg(test)]
    /// Returns true for source words classified as ordinary language keywords.
    pub(crate) fn is_keyword(&self) -> bool {
        self.token_tag().is_keyword()
    }

    #[cfg(test)]
    /// Returns true for word operators (`not`, `is`, `and`, and `or`).
    pub(crate) fn is_word_operator(&self) -> bool {
        self.token_tag().is_word_operator()
    }

    #[cfg(test)]
    /// Returns true for value literal tokens, excluding builtin type spellings.
    pub(crate) fn is_literal(&self) -> bool {
        self.token_tag().is_literal()
    }

    #[cfg(test)]
    /// Returns true for builtin type spellings such as `Int` and `String`.
    pub(crate) fn is_builtin_type(&self) -> bool {
        self.token_tag().is_builtin_type()
    }

    #[cfg(test)]
    /// Returns true for punctuation and structural syntax delimiters.
    pub(crate) fn is_delimiter(&self) -> bool {
        self.token_tag().is_delimiter()
    }

    #[cfg(test)]
    /// Return the AST-compatible precedence for operator tokens.
    pub(crate) fn precedence(&self) -> Option<u8> {
        self.token_tag().precedence()
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
