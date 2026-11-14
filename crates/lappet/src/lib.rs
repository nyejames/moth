//! Native execution and package-management infrastructure for Moth.
//!
//! The crate is split by Cargo feature so each consumer compiles only what it uses:
//!
//! - `runtime` owns native core Wasm execution through Wasmtime. See [`runtime`].
//! - Package management has no code yet. It will arrive behind its own feature and never pull in
//!   Wasmtime.
//!
//! With no feature enabled the crate has no dependency and no API. Host adapters and the Moth
//! host ABI remain design-gated, so nothing here executes compiled Moth yet.

#[cfg(feature = "runtime")]
pub mod runtime;
