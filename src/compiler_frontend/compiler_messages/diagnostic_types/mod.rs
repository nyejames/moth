//! Type-display ids shared by compact diagnostics and the type-display snapshot.
//!
//! WHAT: groups the durable id types that compact diagnostic records store in place of `TypeId`,
//! and the type-display snapshot that renders the spellings those ids name.
//! WHY: the diagnostic store and the type-display snapshot must agree on one id definition, so the
//! id lives in its own module that both depend on.

mod id;
pub(crate) mod type_display_snapshot;
pub(crate) mod type_display_snapshot_builder;

pub(crate) use id::{TypeDisplayCapacityError, TypeDisplayId};

#[cfg(test)]
mod tests;
