//! Compact diagnostic storage: the record vocabulary, the fixed-size store and the two-phase compaction.
//!
//! WHAT: a diagnostic is built as a draft from a family row, checked for capacity, then compacted into fixed-size
//! records held in shared arrays.
//! WHY: one compile error costs a handful of array writes rather than per-diagnostic allocations, and a failed
//! compaction changes nothing, so the store never holds a partly written diagnostic.

pub(crate) mod boundary_accumulator;
mod compaction;
pub(crate) mod diagnostic_string_builder;
pub(crate) mod draft;
mod extras;
mod facts;
pub(crate) mod families;
pub(crate) mod foreign;
pub(crate) mod freeze;
pub(crate) mod labels;
pub(crate) mod prepared;
pub(crate) mod reason_keys;
pub(crate) mod reasons;
mod record;
mod registry;
pub(crate) mod report;
mod resource;
pub(crate) mod schema;
mod store;
pub(crate) mod view;

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
