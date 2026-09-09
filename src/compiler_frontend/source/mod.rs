//! Frontend source identity slots, loaded snapshots and exact compact spans.
//!
//! [`SourceDatabase`] owns the ordered source-slot inventory, its logical-path interner and
//! the dense loaded-record array addressed by each successful slot load. [`SourceId`] provides the
//! compact non-zero identity carried by tokens, headers and references. [`SourceSlot`] carries a
//! four-byte logical path identity, cold filesystem path and load status. [`SourceRecord`] owns the exact UTF-8
//! snapshot and line starts after loading, then receives its frozen extended-span table once the
//! final span-producing stage installs it. [`LocalSpan`] and [`span::SourceSpan`] encode exact half-open
//! UTF-8 byte ranges, while [`line_index`] computes source and tooling line/column positions
//! lazily from a loaded snapshot.
//!
//! Producers that still hold a live extended-span builder use resolver-taking `*_with`
//! operations. Consumers resolve through a frozen record or database with the unqualified
//! record/database operations. This module does not own tokenizer or line/column policy.
//!
//! [`SourceDatabaseBuilder`] is the exclusive build-stage owner that registers, loads and
//! finalizes one database, keeping each loaded source's live span builder until its last
//! producer finishes. [`SourceSpanBuilders<'_>`] is the split-view counterpart a caller holds
//! beside the immutable database borrow when it must keep handing out builders between
//! per-source jobs.
//!
//! - [`id`] defines the four-byte source identity.
//! - [`record`] defines source slots and loaded records.
//! - [`line_index`] owns the borrowed line and column conversions over retained snapshots.
//! - [`database`] owns deterministic lookup, snapshot retention, span-table installation and
//!   traversal-time insertion, plus the exclusive [`SourceDatabaseBuilder`] owner and its
//!   [`SourceSpanBuilders`] split view.
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
#[path = "tests/database_tests.rs"]
mod database_tests;
#[cfg(test)]
#[path = "tests/line_index_tests.rs"]
mod line_index_tests;
#[cfg(test)]
#[path = "tests/render_bridge_tests.rs"]
mod render_bridge_tests;
#[cfg(test)]
#[path = "tests/span_tests.rs"]
mod span_tests;
#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use database::{SourceDatabase, SourceDatabaseBuilder, SourceSpanBuilders};
pub(crate) use id::SourceId;
pub(crate) use record::{SourceKind, SourceProvenance, SourceRecord, SourceSlot};
pub(crate) use registration::SourceRegistrationIndex;
pub(crate) use span::{
    ExtendedSpanBuilder, LocalSpan, SourceSpan, SpanCapacityError, SpanCapacityReason,
};
