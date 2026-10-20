//! Compact diagnostic storage: the record vocabulary, the fixed-size store and the two-phase compaction.
//!
//! WHAT: a diagnostic is built as a draft from a family row, checked for capacity, then compacted into fixed-size
//! records held in shared arrays.
//! WHY: one compile error costs a handful of array writes rather than per-diagnostic allocations, and a failed
//! compaction changes nothing, so the store never holds a partly written diagnostic.

pub(crate) mod compaction;
pub(crate) mod draft;
pub(crate) mod extras;
pub(crate) mod facts;
pub(crate) mod families;
pub(crate) mod foreign;
pub(crate) mod labels;
pub(crate) mod record;
pub(crate) mod registry;
pub(crate) mod resource;
pub(crate) mod schema;
pub(crate) mod store;

#[cfg(test)]
mod tests;
