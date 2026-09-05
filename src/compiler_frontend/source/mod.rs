//! Frontend source identity and path metadata.
//!
//! [`SourceDatabase`] owns the ordered source-record inventory used by frontend preparation, while
//! [`SourceId`] provides the compact non-zero identity carried by tokens, headers and references.
//! Source text, line indexes, spans and the full source-slot/loading lifecycle are introduced by
//! later slices.
//!
//! - [`id`] defines the four-byte source identity.
//! - [`record`] defines one source identity, its provenance and its canonical/logical paths.
//! - [`database`] owns deterministic lookup and traversal-time insertion.

mod database;
mod id;
mod record;
mod registration;

#[cfg(test)]
mod tests;

pub(crate) use database::SourceDatabase;
pub(crate) use id::SourceId;
pub(crate) use record::{SourceProvenance, SourceRecord};
pub(crate) use registration::SourceRegistrationIndex;
