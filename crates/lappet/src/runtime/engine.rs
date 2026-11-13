//! The native engine: one compilation policy shared by every module built from it.

use super::error::RuntimeError;
use super::module::CompiledModule;
use wasmtime::{Config, Engine, Module, Strategy};

/// A Cranelift-backed Wasmtime engine.
///
/// The engine is cheap to share by reference. Modules compiled by one engine can't be used with
/// another.
pub struct RuntimeEngine {
    engine: Engine,
}

impl RuntimeEngine {
    /// Builds the engine, reporting a configuration this host can't support.
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::new();
        // `Strategy::Auto` selects Cranelift today, but an implicit choice isn't a policy.
        config.strategy(Strategy::Cranelift);

        let engine = Engine::new(&config).map_err(RuntimeError::engine_configuration)?;
        Ok(Self { engine })
    }

    /// Validates and compiles binary core Wasm for this engine.
    pub fn compile_module(&self, wasm_bytes: &[u8]) -> Result<CompiledModule, RuntimeError> {
        let module =
            Module::from_binary(&self.engine, wasm_bytes).map_err(RuntimeError::invalid_module)?;
        Ok(CompiledModule::new(module))
    }
}
