//! Exact compact source spans.
//!
//! WHAT: four-byte local spans, eight-byte global spans, one append-only extended table per
//!       source, and one borrowed resolver over both a live builder and a frozen table.
//! WHY:  tokens, diagnostics and syntax must carry exact half-open UTF-8 byte ranges without
//!       scanning source or guessing ends. Inline packing covers the common case; rare long or
//!       late ranges pay for one table entry.
//!
//! This slice does not convert bytes to lines or columns, and it does not store an extended
//! table on [`super::SourceRecord`]. Consumers arrive in later slices. Until then the API is
//! unused in production.

#![allow(dead_code)]

use super::SourceId;
use super::span_encoding::{
    DecodedSpan, decode_logical, encode_extended_index, encode_inline, fits_inline, load_logical,
    store_logical,
};

use std::cmp::Ordering;
use std::mem::size_of;
use std::num::NonZeroU32;

const _: () = assert!(size_of::<LocalSpan>() == 4);
const _: () = assert!(size_of::<Option<LocalSpan>>() == 4);
const _: () = assert!(size_of::<SourceSpan>() == 8);
const _: () = assert!(size_of::<Option<SourceSpan>>() == 8);
const _: () = assert!(size_of::<ExtendedSpan>() == 8);

/// Exact source-local half-open byte range, packed into four bytes.
///
/// A local span carries no source identity, so every resolver, builder and second operand it is
/// given must belong to the source that produced it. Pairing it with another source's extended
/// table resolves to that table's row instead, and nothing at this level can detect that.
/// [`SourceSpan`] is what a consumer crossing a source boundary must hold: it checks that two
/// operands name the same source before joining, overlapping or containing them. The resolver
/// itself carries no identity either, so supplying the wrong table stays the caller's obligation.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalSpan(NonZeroU32);

/// Exact byte range in one identified source.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    source: SourceId,
    local: LocalSpan,
}

/// Source-local overflow row holding an exact start and length that do not fit inline.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExtendedSpan {
    start: u32,
    length: u32,
}

/// Append-only extended-span table for one source that is still producing spans.
#[derive(Debug, Default)]
pub struct ExtendedSpanBuilder {
    entries: Vec<ExtendedSpan>,
}

/// Frozen extended-span table with no spare capacity.
#[derive(Debug)]
pub struct ExtendedSpanTable {
    entries: Box<[ExtendedSpan]>,
}

/// Borrowed view of extended entries used to resolve spans without freezing a live builder.
#[derive(Clone, Copy, Debug)]
pub struct ExtendedSpanResolver<'a> {
    entries: &'a [ExtendedSpan],
}

/// Exact half-open `[start, end)` byte range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedByteRange {
    start: u32,
    end: u32,
}

/// An exact range that cannot be encoded as a `LocalSpan`.
///
/// The caller maps this onto the source-size / source-complexity diagnostic lane. This type
/// only names the offending range and why packing failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanCapacityError {
    start: u32,
    length: u32,
    reason: SpanCapacityReason,
}

/// Why an exact range could not be packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanCapacityReason {
    /// `start + length` is not representable as a `u32` end offset.
    EndUnrepresentable,
    /// The source's extended table cannot address another entry.
    ExtendedTableFull,
}

/// Failure to join two global spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanJoinError {
    DifferentSources { left: SourceId, right: SourceId },
    Capacity(SpanCapacityError),
}

impl LocalSpan {
    /// Encode the exact half-open range `[start, start + length)`.
    ///
    /// Inline packing is used when the frozen 22/10 split can hold both values. Otherwise the
    /// range is appended to `extended` with no deduplication.
    pub fn exact(
        start: u32,
        length: u32,
        extended: &mut ExtendedSpanBuilder,
    ) -> Result<Self, SpanCapacityError> {
        if start.checked_add(length).is_none() {
            return Err(SpanCapacityError::end_unrepresentable(start, length));
        }

        if fits_inline(start, length) {
            let logical = encode_inline(start, length);
            return Ok(Self(store_logical(logical)));
        }

        let Some(logical) = u32::try_from(extended.entries.len())
            .ok()
            .and_then(encode_extended_index)
        else {
            return Err(SpanCapacityError::table_full(start, length));
        };

        extended.entries.push(ExtendedSpan { start, length });

        Ok(Self(store_logical(logical)))
    }

    // The reserved-value test reads the packed logical word without exposing its representation.
    pub(super) fn logical_word(self) -> u32 {
        load_logical(self.0)
    }

    /// Zero-length span at `offset`, used for insertion points, EOF and file-level failures.
    pub fn insertion_point(
        offset: u32,
        extended: &mut ExtendedSpanBuilder,
    ) -> Result<Self, SpanCapacityError> {
        Self::exact(offset, 0, extended)
    }

    /// Resolve to the exact half-open byte range.
    ///
    /// Inline spans are arithmetic only. Extended spans are one bounds-checked lookup. The
    /// resolver may be a live builder or a frozen table.
    pub fn resolve(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange {
        match decode_logical(load_logical(self.0)) {
            DecodedSpan::Inline { start, length } => {
                ResolvedByteRange::from_start_length(start, length)
            }

            DecodedSpan::Extended { index } => {
                let entry = resolver.entries.get(index as usize).expect(
                    "extended span index is outside its source table; this is a compiler bug",
                );

                ResolvedByteRange::from_start_length(entry.start, entry.length)
            }
        }
    }

    pub fn is_empty(self, resolver: ExtendedSpanResolver<'_>) -> bool {
        let range = self.resolve(resolver);
        range.start() == range.end()
    }

    /// Smallest span covering both inputs.
    ///
    /// Both inputs are resolved before any append so the builder borrow stays clean. The cover
    /// is exact and may itself need an extended entry.
    pub fn join(
        self,
        other: Self,
        extended: &mut ExtendedSpanBuilder,
    ) -> Result<Self, SpanCapacityError> {
        let left = self.resolve(extended.resolver());
        let right = other.resolve(extended.resolver());

        let start = left.start().min(right.start());
        let end = left.end().max(right.end());
        let length = end - start;

        Self::exact(start, length, extended)
    }
}

impl SourceSpan {
    pub fn new(source: SourceId, local: LocalSpan) -> Self {
        Self { source, local }
    }

    pub fn source(self) -> SourceId {
        self.source
    }

    pub fn local(self) -> LocalSpan {
        self.local
    }

    pub fn resolve(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange {
        self.local.resolve(resolver)
    }

    pub fn is_empty(self, resolver: ExtendedSpanResolver<'_>) -> bool {
        self.local.is_empty(resolver)
    }

    /// Smallest span covering both inputs.
    ///
    /// Cross-source joins are rejected rather than coerced. Capacity failures from the local
    /// join are preserved as [`SpanJoinError::Capacity`].
    pub fn join(
        self,
        other: Self,
        extended: &mut ExtendedSpanBuilder,
    ) -> Result<Self, SpanJoinError> {
        if self.source != other.source {
            return Err(SpanJoinError::DifferentSources {
                left: self.source,
                right: other.source,
            });
        }

        let local = self
            .local
            .join(other.local, extended)
            .map_err(SpanJoinError::Capacity)?;

        Ok(Self {
            source: self.source,
            local,
        })
    }

    /// Whether the two ranges share at least one byte.
    ///
    /// WHY: overlap is the non-emptiness of the intersection, so a zero-length span overlaps
    /// nothing, not even the range it sits inside. Comparing endpoints pairwise instead would make
    /// an insertion point overlap a range when it is strictly interior and not when it sits on
    /// either edge, so neither answer would mean anything. Use [`Self::contains`] to ask whether
    /// an insertion point lies inside a range, boundaries included.
    ///
    /// Ranges from different sources never overlap.
    pub fn overlaps(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> bool {
        if self.source != other.source {
            return false;
        }

        let left = self.local.resolve(resolver);
        let right = other.local.resolve(resolver);
        let shared_start = left.start().max(right.start());
        let shared_end = left.end().min(right.end());

        shared_start < shared_end
    }

    /// `self` contains `other` when both share a source and `other`'s range lies inside `self`.
    ///
    /// Ranges from different sources are never containment, even when the byte ranges coincide.
    pub fn contains(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> bool {
        if self.source != other.source {
            return false;
        }

        let left = self.local.resolve(resolver);
        let right = other.local.resolve(resolver);

        left.start() <= right.start() && left.end() >= right.end()
    }

    /// Deterministic display order: source identity, then start, then end.
    pub fn source_order(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> Ordering {
        match self.source.cmp(&other.source) {
            Ordering::Equal => {
                let left = self.local.resolve(resolver);
                let right = other.local.resolve(resolver);

                left.start()
                    .cmp(&right.start())
                    .then(left.end().cmp(&right.end()))
            }

            order => order,
        }
    }
}

impl ExtendedSpanBuilder {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn resolver(&self) -> ExtendedSpanResolver<'_> {
        ExtendedSpanResolver {
            entries: &self.entries,
        }
    }

    pub fn freeze(self) -> ExtendedSpanTable {
        ExtendedSpanTable {
            entries: self.entries.into_boxed_slice(),
        }
    }
}

impl ExtendedSpanTable {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn resolver(&self) -> ExtendedSpanResolver<'_> {
        ExtendedSpanResolver {
            entries: &self.entries,
        }
    }
}

impl ResolvedByteRange {
    fn from_start_length(start: u32, length: u32) -> Self {
        let end = start
            .checked_add(length)
            .expect("encoded span end overflowed u32; this is a compiler bug");

        Self { start, end }
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn end(self) -> u32 {
        self.end
    }
}

impl SpanCapacityError {
    fn end_unrepresentable(start: u32, length: u32) -> Self {
        Self {
            start,
            length,
            reason: SpanCapacityReason::EndUnrepresentable,
        }
    }

    fn table_full(start: u32, length: u32) -> Self {
        Self {
            start,
            length,
            reason: SpanCapacityReason::ExtendedTableFull,
        }
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn length(self) -> u32 {
        self.length
    }

    pub fn reason(self) -> SpanCapacityReason {
        self.reason
    }
}
