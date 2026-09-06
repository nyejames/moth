//! Exact compact source spans.
//!
//! WHAT: four-byte local spans, eight-byte global spans, one append-only extended table per
//!       source, and one borrowed resolver over both a live builder and a frozen table.
//! WHY:  tokens, diagnostics and syntax must carry exact half-open UTF-8 byte ranges without
//!       scanning source or guessing ends. Inline packing covers the common case; rare long or
//!       late ranges pay for one table entry.
//!
//! Producers that still append to a source's builder use the resolver-taking `*_with` operations.
//! Consumers use the unqualified record and database operations after the builder is installed
//! into a frozen [`super::SourceRecord`]. This module does not convert bytes to lines or columns.

#![allow(dead_code)]

use super::span_encoding::{
    DecodedSpan, decode_logical, encode_extended_index, encode_inline, fits_inline, load_logical,
    store_logical,
};
use super::{SourceDatabase, SourceId, SourceRecord};

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
/// table resolves to that table's row instead, and no resolver can detect that. Database-backed
/// resolution knows the identity it looked the record up by, so it can name that source when a
/// span reaches an absent or too-short table; resolving through a bare record cannot, because a
/// [`SourceRecord`] carries no identity. [`SourceSpan`] is what a consumer crossing a source
/// boundary must hold: it checks that two operands name the same source before joining,
/// overlapping or containing them.
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
    /// A live builder and a frozen table both present their entries here. Absent means the
    /// source's builder was never installed on its loaded record, which is a producer bug rather
    /// than a source with no extended spans: that source has an installed empty table.
    entries: Option<&'a [ExtendedSpan]>,
    /// Names the source when the caller knows its identity, so a compiler-bug panic can point at
    /// it. A `SourceRecord` carries no identity of its own, so the record-only form has none.
    source_identity: Option<SourceId>,
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

fn record_resolver(
    source: &SourceRecord,
    source_identity: Option<SourceId>,
) -> ExtendedSpanResolver<'_> {
    ExtendedSpanResolver {
        entries: source
            .extended_spans
            .as_ref()
            .map(|table| table.entries.as_ref()),
        source_identity,
    }
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

    /// Resolve through a loaded record's frozen extended-span table.
    ///
    /// Inline spans remain valid before the builder is installed. An extended span reaching an
    /// absent table is a compiler bug because its producer failed to install that builder.
    pub fn resolve(self, source: &SourceRecord) -> ResolvedByteRange {
        self.resolve_with(record_resolver(source, None))
    }

    /// Resolve with a producer's live builder or a frozen table resolver.
    pub fn resolve_with(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange {
        match decode_logical(load_logical(self.0)) {
            DecodedSpan::Inline { start, length } => {
                ResolvedByteRange::from_start_length(start, length)
            }

            DecodedSpan::Extended { index } => {
                let entry = resolver
                    .extended_entry(index)
                    .unwrap_or_else(|| resolver.report_unresolvable_index(index));

                ResolvedByteRange::from_start_length(entry.start, entry.length)
            }
        }
    }

    pub fn is_empty(self, source: &SourceRecord) -> bool {
        let range = self.resolve(source);
        range.start() == range.end()
    }

    pub fn is_empty_with(self, resolver: ExtendedSpanResolver<'_>) -> bool {
        let range = self.resolve_with(resolver);
        range.start() == range.end()
    }

    fn resolve_from_record(
        self,
        source: &SourceRecord,
        source_identity: SourceId,
    ) -> ResolvedByteRange {
        self.resolve_with(record_resolver(source, Some(source_identity)))
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
        let left = self.resolve_with(extended.resolver());
        let right = other.resolve_with(extended.resolver());

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

    pub fn resolve_with(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange {
        self.local.resolve_with(resolver)
    }

    pub fn is_empty_with(self, resolver: ExtendedSpanResolver<'_>) -> bool {
        self.local.is_empty_with(resolver)
    }

    pub fn byte_range(self, sources: &SourceDatabase) -> ResolvedByteRange {
        let source = sources.source_record(self.source);
        self.local.resolve_from_record(source, self.source)
    }

    pub fn start(self, sources: &SourceDatabase) -> u32 {
        self.byte_range(sources).start()
    }

    pub fn end(self, sources: &SourceDatabase) -> u32 {
        self.byte_range(sources).end()
    }

    pub fn overlaps(self, other: Self, sources: &SourceDatabase) -> bool {
        if self.source != other.source {
            return false;
        }

        let source = sources.source_record(self.source);
        self.overlaps_with(other, record_resolver(source, Some(self.source)))
    }

    pub fn contains(self, other: Self, sources: &SourceDatabase) -> bool {
        if self.source != other.source {
            return false;
        }

        let source = sources.source_record(self.source);
        self.contains_with(other, record_resolver(source, Some(self.source)))
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
    /// either edge, so neither answer would mean anything. Use [`Self::contains_with`] to ask
    /// whether an insertion point lies inside a range, boundaries included.
    ///
    /// Ranges from different sources never overlap.
    pub fn overlaps_with(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> bool {
        if self.source != other.source {
            return false;
        }

        let left = self.local.resolve_with(resolver);
        let right = other.local.resolve_with(resolver);
        let shared_start = left.start().max(right.start());
        let shared_end = left.end().min(right.end());

        shared_start < shared_end
    }

    /// `self` contains `other` when both share a source and `other`'s range lies inside `self`.
    ///
    /// Ranges from different sources are never containment, even when the byte ranges coincide.
    pub fn contains_with(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> bool {
        if self.source != other.source {
            return false;
        }

        let left = self.local.resolve_with(resolver);
        let right = other.local.resolve_with(resolver);

        left.start() <= right.start() && left.end() >= right.end()
    }

    /// Deterministic display order: source identity, then start, then end.
    pub fn source_order_with(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> Ordering {
        match self.source.cmp(&other.source) {
            Ordering::Equal => {
                let left = self.local.resolve_with(resolver);
                let right = other.local.resolve_with(resolver);

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
            entries: Some(&self.entries),
            source_identity: None,
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
            entries: Some(&self.entries),
            source_identity: None,
        }
    }
}

impl<'a> ExtendedSpanResolver<'a> {
    fn extended_entry(self, index: u32) -> Option<&'a ExtendedSpan> {
        self.entries?.get(index as usize)
    }

    /// Report an extended index this resolver cannot resolve.
    ///
    /// WHY: both causes are compiler bugs, but they are different bugs. An absent table means the
    /// producer never installed its builder on the loaded record, so no index could resolve. An
    /// index past an installed table means the span was encoded against a table that is not this
    /// one, whether another source's or an earlier builder for this source.
    #[cold]
    fn report_unresolvable_index(self, index: u32) -> ! {
        let source = match self.source_identity {
            Some(identity) => format!("source identity {}", identity.index()),
            None => "this source record".to_owned(),
        };

        match self.entries {
            Some(entries) => panic!(
                "extended span index {index} is outside the {} extended spans of {source}; \
                 this is a compiler bug",
                entries.len()
            ),
            None => panic!(
                "extended span builder for {source} was never installed; this is a compiler bug"
            ),
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
