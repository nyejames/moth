//! Compiled modules: immutable native code for one engine.

use wasmtime::Module;

/// Binary core Wasm validated and compiled to native code for one [`super::RuntimeEngine`].
///
/// A compiled module holds no guest state, so any number of instances may be created from it.
/// Cloning shares the compiled code and never recompiles it.
#[derive(Clone)]
pub struct CompiledModule {
    module: Module,
}

impl CompiledModule {
    pub(super) fn new(module: Module) -> Self {
        Self { module }
    }

    pub(super) fn wasmtime(&self) -> &Module {
        &self.module
    }
}
