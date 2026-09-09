//! Exact compact source spans.
//!
//! WHAT: four-byte local spans, eight-byte global spans, one append-only extended table per
//!       source, and one borrowed resolver over both a live builder and a frozen table.
//! WHY:  tokens, diagnostics and syntax must carry exact half-open UTF-8 byte ranges without
//!       scanning source or guessing ends. Inline packing covers the common case; rare long or
//!       late ranges pay for one table entry.
//!
//! Producers append to one source's builder and resolve their own local spans through its bare
//! resolver. Consumers use the record and database operations after the builder is installed into
//! a frozen [`super::SourceRecord`]. A global [`SourceSpan`] operation accepts only a resolver
//! qualified for the span's own source and rejects every other pairing as a compiler bug. This
//! module does not convert bytes to lines or columns.
//!

use super::span_encoding::{
    DecodedSpan, decode_logical, encode_extended_index, encode_inline, fits_inline, load_logical,
    store_logical,
};
use super::{FrozenIdentityContext, FrozenSourceDatabase, SourceDatabase, SourceId, SourceRecord};

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
///
/// The source identity is what makes resolution checkable: a global span may only resolve through
/// a resolver qualified for [`SourceSpan::source`], and every `*_with` operation rejects another
/// pairing instead of silently reading a wrong table row.
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
    /// Read through [`ExtendedSpanTable::resolver`] and [`record_resolver`]; both reach
    /// production when slice 1D4 installs a table on a loaded record.
    #[allow(dead_code)]
    entries: Box<[ExtendedSpan]>,
}

/// Borrowed view of extended entries used to resolve spans without freezing a live builder.
///
/// A resolver is either bare, for the source-local [`LocalSpan`] work of the source that owns the
/// entries, or qualified through [`Self::for_source`], required by every global [`SourceSpan`]
/// operation.
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
/// only names the offending range and why packing failed. Exhaustion of the extended table
/// ([`SpanCapacityReason::ExtendedTableFull`]) is terminal for the producing stage: the exact
/// range must survive in the mapped diagnostic and never be silently dropped.
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
    /// The source's extended table cannot address another entry. Exhaustion is terminal for
    /// the producing stage; the failing exact range must surface in the mapped diagnostic.
    ExtendedTableFull,
}

/// Failure to join two global spans.
///
/// Returned by [`SourceSpan::join`], whose first caller is the diagnostic migration of slice 1D3.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanJoinError {
    DifferentSources { left: SourceId, right: SourceId },
    Capacity(SpanCapacityError),
}

/// Lookup boundary used by exact global-span resolution.
///
/// Both mutable and frozen source owners implement this narrow source-record lookup, so a span
/// keeps one exact resolution API while the owner lifecycle changes at the identity boundary.
pub(crate) trait SourceSpanDatabase {
    fn source_record_for_span(&self, source: SourceId) -> &SourceRecord;
}

impl SourceSpanDatabase for SourceDatabase {
    fn source_record_for_span(&self, source: SourceId) -> &SourceRecord {
        self.source_record(source)
    }
}

impl SourceSpanDatabase for FrozenSourceDatabase {
    fn source_record_for_span(&self, source: SourceId) -> &SourceRecord {
        self.source_record(source)
    }
}

impl SourceSpanDatabase for FrozenIdentityContext {
    fn source_record_for_span(&self, source: SourceId) -> &SourceRecord {
        self.source_record(source)
    }
}

/// A record's own resolver, available once slice 1D4 installs the source's frozen table.
#[allow(dead_code)]
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
    /// The empty span at byte offset zero, for a synthetic token or anchor with no authored text.
    ///
    /// This is the one span a caller can name without the source's builder, because it always
    /// packs inline. It goes through the codec rather than a literal so the packing stays owned
    /// by `span_encoding`.
    pub fn source_start() -> Self {
        Self(store_logical(encode_inline(0, 0)))
    }

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
}

/// The consumer half of a local span.
///
/// WHY: resolving against a `SourceRecord` needs the table slice 1D4 installs there, and
/// insertion points, emptiness and joins get their first callers in the diagnostic, syntax and
/// renderer migrations of slices 1D3, 1E and 1F. This module's tests exercise every operation
/// here; each method below therefore carries its own narrow allowance naming unreached callers
/// rather than unproven code.
impl LocalSpan {
    // The reserved-value test reads the packed logical word without exposing its representation.
    pub(super) fn logical_word(self) -> u32 {
        load_logical(self.0)
    }

    /// Zero-length span at `offset`, used for insertion points, EOF and file-level failures.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn resolve(self, source: &SourceRecord) -> ResolvedByteRange {
        self.resolve_with(record_resolver(source, None))
    }

    #[allow(dead_code)]
    pub fn is_empty(self, source: &SourceRecord) -> bool {
        let range = self.resolve(source);
        range.start() == range.end()
    }

    #[allow(dead_code)]
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

/// Global spans reach production with the diagnostics of slice 1D3, the first values that cross
/// a source boundary. This module's tests prove every operation below.
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

    /// Resolve through a resolver qualified for this span's own source.
    ///
    /// # Panics
    /// Panics when the resolver claims a different source or claims none: a global span may
    /// never resolve through another source's extended table, because that reads a row of
    /// foreign source bytes.
    #[allow(dead_code)]
    pub fn resolve_with(self, resolver: ExtendedSpanResolver<'_>) -> ResolvedByteRange {
        self.reject_foreign_resolver(resolver);
        self.local.resolve_with(resolver)
    }

    /// Emptiness through a resolver qualified for this span's own source.
    ///
    /// # Panics
    /// Panics under the same wrong-source pairing as [`Self::resolve_with`].
    #[allow(dead_code)]
    pub fn is_empty_with(self, resolver: ExtendedSpanResolver<'_>) -> bool {
        self.reject_foreign_resolver(resolver);
        self.local.is_empty_with(resolver)
    }

    /// Resolve against the source owner that owns this span's identity.
    ///
    /// Mutable [`SourceDatabase`] and lookup-only [`FrozenSourceDatabase`] owners share this exact
    /// operation. The reserved compilation root owns no record, so its spans resolve without a
    /// lookup and only through its exact empty range; every other identity resolves through its
    /// own loaded record, and an unresolvable extended row names that source as a compiler bug.
    pub fn byte_range<S: SourceSpanDatabase>(self, sources: &S) -> ResolvedByteRange {
        if self.source == SourceId::COMPILATION_ROOT {
            return self.resolve_compilation_root_range();
        }

        let source = sources.source_record_for_span(self.source);
        self.local.resolve_from_record(source, self.source)
    }

    /// Resolve the one range the reserved compilation root can resolve against the database.
    ///
    /// # Panics
    /// Panics for any root range other than the exact empty `[0, 0)`. The root owns no snapshot
    /// and no extended table, so a non-empty root span names bytes no source owns. That range is
    /// a proven-impossible misuse: a compiler invariant failure, never a silent wrong
    /// resolution.
    fn resolve_compilation_root_range(self) -> ResolvedByteRange {
        match decode_logical(self.local.logical_word()) {
            DecodedSpan::Inline {
                start: 0,
                length: 0,
            } => ResolvedByteRange::from_start_length(0, 0),
            DecodedSpan::Inline { start, length } => panic!(
                "span on compilation root source identity {} resolves only the exact empty \
                 range [0, 0), not [{start}, {}); this is a compiler bug",
                self.source.index(),
                start + length
            ),
            DecodedSpan::Extended { index } => panic!(
                "span on compilation root source identity {} names extended row {index}, but the \
                 root owns no extended-span table; only the exact empty range [0, 0) resolves on \
                 the root; this is a compiler bug",
                self.source.index()
            ),
        }
    }

    #[allow(dead_code)]
    pub fn start<S: SourceSpanDatabase>(self, sources: &S) -> u32 {
        self.byte_range(sources).start()
    }

    #[allow(dead_code)]
    pub fn end<S: SourceSpanDatabase>(self, sources: &S) -> u32 {
        self.byte_range(sources).end()
    }

    #[allow(dead_code)]
    pub fn overlaps<S: SourceSpanDatabase>(self, other: Self, sources: &S) -> bool {
        if self.source != other.source {
            return false;
        }

        let left = self.byte_range(sources);
        let right = other.byte_range(sources);
        left.start().max(right.start()) < left.end().min(right.end())
    }

    #[allow(dead_code)]
    pub fn contains<S: SourceSpanDatabase>(self, other: Self, sources: &S) -> bool {
        if self.source != other.source {
            return false;
        }

        let outer = self.byte_range(sources);
        let inner = other.byte_range(sources);
        outer.start() <= inner.start() && outer.end() >= inner.end()
    }

    /// Smallest span covering both inputs.
    ///
    /// Cross-source joins are rejected rather than coerced. Capacity failures from the local
    /// join are preserved as [`SpanJoinError::Capacity`].
    #[allow(dead_code)]
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
    ///
    /// # Panics
    /// Panics when the resolver is not qualified for this span's source. Cross-source operands
    /// still return `false` without resolving anything.
    #[allow(dead_code)]
    pub fn overlaps_with(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> bool {
        if self.source != other.source {
            return false;
        }

        self.reject_foreign_resolver(resolver);
        let left = self.local.resolve_with(resolver);
        let right = other.local.resolve_with(resolver);
        let shared_start = left.start().max(right.start());
        let shared_end = left.end().min(right.end());

        shared_start < shared_end
    }

    /// `self` contains `other` when both share a source and `other`'s range lies inside `self`.
    ///
    /// Ranges from different sources are never containment, even when the byte ranges coincide.
    ///
    /// # Panics
    /// Panics when the resolver is not qualified for this span's source. Cross-source operands
    /// still return `false` without resolving anything.
    #[allow(dead_code)]
    pub fn contains_with(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> bool {
        if self.source != other.source {
            return false;
        }

        self.reject_foreign_resolver(resolver);
        let left = self.local.resolve_with(resolver);
        let right = other.local.resolve_with(resolver);

        left.start() <= right.start() && left.end() >= right.end()
    }

    /// Deterministic display order: source identity, then start, then end.
    ///
    /// # Panics
    /// Panics when the resolver is not qualified for this span's source. Cross-source operands
    /// order by identity without resolving anything.
    #[allow(dead_code)]
    pub fn source_order_with(self, other: Self, resolver: ExtendedSpanResolver<'_>) -> Ordering {
        match self.source.cmp(&other.source) {
            Ordering::Equal => {
                self.reject_foreign_resolver(resolver);
                let left = self.local.resolve_with(resolver);
                let right = other.local.resolve_with(resolver);

                left.start()
                    .cmp(&right.start())
                    .then(left.end().cmp(&right.end()))
            }

            order => order,
        }
    }

    /// Reject a resolver that does not claim this span's source.
    ///
    /// WHY: the span's local indexes were encoded against its own source's table; a foreign or
    /// unqualified resolver would return a row of different source bytes. This is a compiler
    /// invariant failure, not a recoverable condition.
    #[allow(dead_code)]
    fn reject_foreign_resolver(self, resolver: ExtendedSpanResolver<'_>) {
        match resolver.source_identity {
            Some(identity) if identity == self.source => {}
            Some(identity) => panic!(
                "span from source identity {} cannot resolve through source identity {}; \
                 this is a compiler bug",
                self.source.index(),
                identity.index()
            ),
            None => panic!(
                "span from source identity {} resolved through an unqualified resolver; \
                 global spans require resolver_for(source); this is a compiler bug",
                self.source.index()
            ),
        }
    }
}

impl ExtendedSpanBuilder {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// The bare resolver for this builder's own source-local work.
    pub fn resolver(&self) -> ExtendedSpanResolver<'_> {
        ExtendedSpanResolver {
            entries: Some(&self.entries),
            source_identity: None,
        }
    }

    /// The resolver qualified for `source`, required by global [`SourceSpan`] operations.
    ///
    /// WHY: a builder belongs to one source while it produces spans; qualifying at the builder
    /// names that source once instead of at every operation. Producers resolve their own local
    /// spans, so the qualified form first reaches compilation with the consumer migrations.
    #[allow(dead_code)]
    pub fn resolver_for(&self, source: SourceId) -> ExtendedSpanResolver<'_> {
        self.resolver().for_source(source)
    }

    pub fn freeze(self) -> ExtendedSpanTable {
        ExtendedSpanTable {
            entries: self.entries.into_boxed_slice(),
        }
    }
}

#[cfg(test)]
impl ExtendedSpanBuilder {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// Frozen-table readers switch over with the source-span migration in slices 1D3/1E.
impl ExtendedSpanTable {
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The bare resolver for this table's own source-local work.
    #[allow(dead_code)]
    pub fn resolver(&self) -> ExtendedSpanResolver<'_> {
        ExtendedSpanResolver {
            entries: Some(&self.entries),
            source_identity: None,
        }
    }

    /// The resolver qualified for `source`, required by global [`SourceSpan`] operations.
    ///
    /// The table is installed on a record whose source is known at the install site.
    #[allow(dead_code)]
    pub fn resolver_for(&self, source: SourceId) -> ExtendedSpanResolver<'_> {
        self.resolver().for_source(source)
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

    /// Qualify this resolver as belonging to `source`.
    ///
    /// WHY: a global [`SourceSpan`] names its source, so the resolver that serves it must claim
    /// the same one. Qualification turns the wrong-source pairing into a deterministic compiler
    /// invariant failure at the operation instead of a silent wrong-row read.
    pub fn for_source(self, source: SourceId) -> Self {
        Self {
            source_identity: Some(source),
            ..self
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
