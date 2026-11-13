//! Compiled modules: immutable native code for one engine.

use wasmtime::Module;

/// Binary core Wasm validated and compiled to native code for one [`super::RuntimeEngine`].
///
/// A compiled module holds no guest state, so any number of instances may be created from it.
pub struct CompiledModule {
    // Read once instances exist. Until then the field only keeps the compiled code alive.
    _module: Module,
}

impl CompiledModule {
    pub(super) fn new(module: Module) -> Self {
        Self { _module: module }
    }
}
