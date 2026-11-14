//! Native core Wasm execution through Wasmtime's Cranelift compiler.
//!
//! This module owns the engine, compiled modules, isolated instances, their limits and the
//! classified errors every runtime failure is reported through. It establishes runtime
//! mechanics: it drives the reserved call-scope helper names and does not know a host
//! descriptor or a compiled Moth artefact.
//!
//! Data flows one way:
//!
//! 1. [`RuntimeEngine`] (`engine.rs`) fixes the native compilation policy for everything built
//!    from it.
//! 2. [`CompiledModule`] (`module.rs`) is binary core Wasm validated and compiled for one
//!    engine. It is immutable and may be shared.
//! 3. `HostImports` (`imports.rs`) registers host functions by physical Wasm signature and
//!    checks a module's imports once, producing a `PreparedModule`.
//! 4. `Instance` (`instance.rs`) owns one store, one guest and one host state under
//!    [`InstanceLimits`] (`limits.rs`). Its `call` is the guest entry and owns quarantine.
//!    Context moves into that store for the call and back out before it returns. A scoped
//!    export call drives the native host ABI's scope helpers and skips close after a fault.
//! 5. `fault.rs` classifies a failed host import or guest entry into a [`RuntimeError`]
//!    (`error.rs`).
//!
//! Steps 3 and 4 are crate-private. Installing raw imports and calling raw exports is the
//! mechanism the host adapters build on, not an embedding API. The public call surface arrives
//! with the manifest-driven adapters, so no raw-Wasm API is published by accident.
//!
//! Wasmtime types never appear in a public signature, so the pinned release can change without
//! changing this API. Production code compiles binary Wasm only. WAT is a test-fixture format.
//! There is no `unsafe`, WASI, component model, async execution or compiled-code cache here.

// The crate-private instance surface has no production caller until the host adapters arrive.
// Tests are its only consumer today, and a test build still reports code nothing reaches.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the raw instance surface waits for the host adapters and is exercised by tests"
    )
)]

mod context;
mod engine;
mod error;
mod fault;
mod imports;
mod instance;
mod limits;
mod module;
mod resource;
mod staging;

#[cfg(test)]
mod tests;

pub use engine::{NativeOptimisation, RuntimeEngine};
pub use error::{BoundaryFault, ContextFault, HostFault, LimitFault, LinkError, RuntimeError};
pub use limits::InstanceLimits;
pub use module::CompiledModule;
