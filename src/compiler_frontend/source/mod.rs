//! Frontend source identity, retained snapshots, path metadata and exact compact spans.
//!
//! [`SourceDatabase`] owns the ordered source-record inventory used by frontend preparation, while
//! [`SourceId`] provides the compact non-zero identity carried by tokens, headers and references.
//! Physical source records own the exact UTF-8 snapshots used during compilation. [`LocalSpan`]
//! and [`SourceSpan`] encode exact half-open UTF-8 byte ranges; line and column conversion is
//! still outside this slice, and extended tables are not stored on source records yet.
//!
//! - [`id`] defines the four-byte source identity.
//! - [`record`] defines one source identity, its retained text, loading status and paths.
//! - [`database`] owns deterministic lookup, snapshot retention and traversal-time insertion.
//! - [`span`] defines exact local and global spans, the per-source extended table and the
//!   shared resolver over a live builder and a frozen table.
//! - [`span_encoding`] is the private codec for the frozen 22/10 split. It has no surface
//!   outside this module.

mod database;
mod id;
mod record;
mod registration;
mod span;
mod span_encoding;

#[cfg(test)]
mod tests;

pub(crate) use database::SourceDatabase;
pub(crate) use id::SourceId;
pub(crate) use record::{SourceKind, SourceProvenance, SourceRecord};
pub(crate) use registration::SourceRegistrationIndex;
// Slice 1D is the first production consumer. Re-exporting now would otherwise look unused
// in a lib-only build.
#[allow(unused_imports)]
pub(crate) use span::{
    ExtendedSpan, ExtendedSpanBuilder, ExtendedSpanResolver, ExtendedSpanTable, LocalSpan,
    ResolvedByteRange, SourceSpan, SpanCapacityError, SpanCapacityReason, SpanJoinError,
};
