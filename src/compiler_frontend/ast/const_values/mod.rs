//! Shared AST const value resolver and fact types.
//!
//! WHAT: owns declaration const facts and the resolution logic that determines
//!       whether an AST expression is a compile-time constant.
//! WHY: config validation, AST finalization, and HIR metadata all need one
//!      shared source of truth for const-ness instead of duplicating evaluation
//!      logic or adding config-specific scanners.
//!
//! ## Design invariants
//!
//! - Private const facts are internal compiler metadata. They are not syntax
//!   and are not exposed in user-facing diagnostics.
//! - Private const facts do not affect dependency sorting or visibility.
//! - Config is one consumer of these facts, not the owner.
//! - Explicit `#` constants keep their existing top-level dependency/sorting
//!   semantics; this module does not replace that path.

pub(crate) mod facts;
pub(crate) mod resolver;
pub(crate) mod store;

#[cfg(test)]
mod tests;
