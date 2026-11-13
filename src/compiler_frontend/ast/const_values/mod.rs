//! Shared AST const value resolution and storage.
//!
//! WHAT: owns the authoritative folded constant store and the resolution logic that determines
//!       whether an AST expression is a compile-time constant.
//! WHY: config validation, AST finalization, and HIR lowering all need one shared source of truth
//!      for const-ness instead of duplicating evaluation logic or adding config-specific scanners.
//!      Resolved expressions remain transient in the lexical environment or authoritative store.
//!
//! ## Design invariants
//!
//! - Resolved private constants are internal compiler values. They are not syntax and are not
//!   exposed in user-facing diagnostics.
//! - Private constants do not affect dependency sorting or visibility.
//! - Explicit `#` constants keep their existing top-level dependency/sorting semantics; this
//!   module does not replace that path.

pub(crate) mod binding;
pub(crate) mod const_execution;
pub(crate) mod resolver;
pub(crate) mod store;

pub(crate) use store::{ConstValueId, ConstValuePayload, ConstValueStore, ConstValueStoreError};

#[cfg(test)]
mod tests;
