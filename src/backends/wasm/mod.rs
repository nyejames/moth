//! Experimental Wasm backend entry points.
//!
//! WHAT: owns HIR -> Wasm LIR lowering, optional core Wasm emission, runtime helper contracts,
//! and debug output for the HTML/Wasm experiment.
//! WHY: the backend remains crate-internal because it is still an experimental path.
pub(crate) mod backend;
pub(crate) mod debug;
pub(crate) mod emit;
pub(crate) mod hir_to_lir;
pub(crate) mod lir;
pub(crate) mod request;
pub(crate) mod result;
pub(crate) mod runtime;

#[cfg(test)]
mod tests;
