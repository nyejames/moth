//! Frontend source identity and path metadata.
//!
//! [`SourceDatabase`] owns the ordered source-record inventory used by frontend preparation, while
//! [`SourceId`] provides the compact non-zero identity carried by tokens, headers and references.
//! This module intentionally excludes source text, line indexes, spans, path-ID migration and the
//! build-lifetime registration barrier; those belong to later slices.
//!
//! - [`id`] defines the four-byte source identity.
//! - [`record`] defines one source identity and its canonical/logical paths.
//! - [`database`] owns deterministic lookup and traversal-time insertion.

mod database;
mod id;
mod record;

#[cfg(test)]
mod tests;

pub(crate) use database::SourceDatabase;
pub(crate) use id::SourceId;
pub(crate) use record::SourceRecord;
