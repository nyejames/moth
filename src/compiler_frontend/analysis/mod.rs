//! Frontend semantic analysis entry points and shared result types.
//!
//! WHAT: owns the side analyses that read validated HIR without rewriting it: the mandatory
//!       borrow checker and the optional numeric proof table.
//! WHY: borrow-checking reports feed module compilation, summary convergence and the Wasm
//!      backend's ownership lowering, while numeric proofs are read only by the JS and Wasm
//!      backends to elide proven-redundant range checks. Both consume the same validated HIR, so
//!      one module owns them.

pub(crate) mod borrow_checker;
pub(crate) mod numeric_proofs;
