//! Frontend source identity, retained snapshots and path metadata.
//!
//! [`SourceDatabase`] owns the ordered source-record inventory used by frontend preparation, while
//! [`SourceId`] provides the compact non-zero identity carried by tokens, headers and references.
//! Physical source records own the exact UTF-8 snapshots used during compilation; line indexes and
//! spans remain outside this slice.
//!
//! - [`id`] defines the four-byte source identity.
//! - [`record`] defines one source identity, its retained text, loading status and paths.
//! - [`database`] owns deterministic lookup, snapshot retention and traversal-time insertion.

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
