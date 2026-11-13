//! Native core Wasm execution through Wasmtime's Cranelift compiler.
//!
//! This module owns the engine, compiled modules and the classified errors every runtime
//! failure is reported through. It establishes runtime mechanics only: nothing here knows the
//! Moth host ABI, a host descriptor or a compiled Moth artefact.
//!
//! Data flows one way:
//!
//! 1. [`RuntimeEngine`] fixes the native compilation policy for everything built from it.
//! 2. [`CompiledModule`] is binary core Wasm validated and compiled for one engine. It is
//!    immutable and may be shared.
//!
//! Wasmtime types never appear in a public signature, so the pinned release can change without
//! changing this API. Production code compiles binary Wasm only. WAT is a test-fixture format.

mod engine;
mod error;
mod module;

#[cfg(test)]
mod tests;

pub use engine::RuntimeEngine;
pub use error::RuntimeError;
pub use module::CompiledModule;
