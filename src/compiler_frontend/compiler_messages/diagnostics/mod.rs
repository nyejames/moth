//! Compact diagnostic storage: the record vocabulary, the fixed-size store and the two-phase compaction.
//!
//! WHAT: a diagnostic is built as a draft from a family row, checked for capacity, then compacted into fixed-size
//! records held in shared arrays.
//! WHY: one compile error costs a handful of array writes rather than per-diagnostic allocations, and a failed
//! compaction changes nothing, so the store never holds a partly written diagnostic.

mod compaction;
mod draft;
mod extras;
mod facts;
pub(crate) mod families;
pub(crate) mod foreign;
mod freeze;
pub(crate) mod labels;
pub(crate) mod legacy_adapter;
mod record;
mod registry;
pub(crate) mod report;
mod resource;
mod schema;
mod store;
pub(crate) mod view;

#[cfg(test)]
mod tests;
