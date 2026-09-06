//! Frontend source identity slots, loaded snapshots, retained text and exact compact spans.
//!
//! [`SourceDatabase`] owns the ordered source-slot inventory used by frontend preparation, plus
//! the dense loaded-record array addressed by each successful slot load. [`SourceId`] provides the
//! compact non-zero identity carried by tokens, headers and references. [`SourceSlot`] owns every
//! candidate's identity, path metadata and load status; [`SourceRecord`] owns the exact UTF-8
//! snapshot and line starts only after that candidate loads. [`LocalSpan`] and [`SourceSpan`]
//! encode exact half-open UTF-8 byte ranges, while [`line_index`] computes source and tooling
//! line/column positions lazily from a loaded snapshot.
//!
//! - [`id`] defines the four-byte source identity.
//! - [`record`] defines source slots and loaded records.
//! - [`line_index`] owns the borrowed line and column conversions over retained snapshots.
//! - [`database`] owns deterministic lookup, snapshot retention and traversal-time insertion.
//! - [`span`] defines exact local and global spans, the per-source extended table and the
//!   shared resolver over a live builder and a frozen table.
//! - [`span_encoding`] is the private codec for the frozen 22/10 split. It has no surface
//!   outside this module.

mod database;
mod id;
pub(crate) mod line_index;
mod record;
mod registration;
mod span;
mod span_encoding;

#[cfg(test)]
mod tests;

pub(crate) use database::SourceDatabase;
pub(crate) use id::SourceId;
pub(crate) use record::{SourceKind, SourceProvenance, SourceRecord, SourceSlot};
pub(crate) use registration::SourceRegistrationIndex;
// Slice 1D is the first production consumer. Re-exporting now would otherwise look unused
// in a lib-only build.
#[allow(unused_imports)]
pub(crate) use span::{
    ExtendedSpan, ExtendedSpanBuilder, ExtendedSpanResolver, ExtendedSpanTable, LocalSpan,
    ResolvedByteRange, SourceSpan, SpanCapacityError, SpanCapacityReason, SpanJoinError,
};
