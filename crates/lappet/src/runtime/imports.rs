//! Raw host import registration, checked once against a compiled module.
//!
//! This is the crate-private mechanism the host adapters will build on. An import is named and
//! typed by its physical Wasm signature here. Canonical identities, descriptors and value
//! conversion belong to the adapters.
//!
//! A host import receives its instance's host state and the call's Wasm values, and nothing
//! else. It gets no handle that could call an export, so it can't re-enter the guest.

use super::engine::RuntimeEngine;
use super::error::{ErrorSource, LinkError, RuntimeError};
use super::fault::run_host_import;
use super::instance::{PreparedModule, StoreData};
use super::module::CompiledModule;
use wasmtime::{Engine, FuncType, Linker, UnknownImportError, Val, ValType};

/// The host imports one kind of instance provides, before they're checked against a module.
///
/// Registration is store-independent, so one set of imports serves every instance prepared
/// from it. Mutable state lives in each instance's `HostState`, never in the import closures.
pub(crate) struct HostImports<HostState: 'static> {
    linker: Linker<StoreData<HostState>>,
}

impl<HostState: 'static> HostImports<HostState> {
    pub(crate) fn new(engine: &RuntimeEngine) -> Self {
        // A new linker rejects a second definition of one name and defines nothing by itself.
        // An import the host didn't register is therefore always a link error, never a stub.
        Self {
            linker: Linker::new(engine.wasmtime()),
        }
    }

    /// Registers one host function under its physical Wasm signature.
    ///
    /// The function fills `results` in order. A failure it returns, or a panic, becomes a host
    /// fault that ends the guest call.
    pub(crate) fn define(
        &mut self,
        module: &str,
        name: &str,
        parameters: impl IntoIterator<Item = ValType>,
        results: impl IntoIterator<Item = ValType>,
        host_function: impl Fn(&mut HostState, &[Val], &mut [Val]) -> Result<(), ErrorSource>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), RuntimeError> {
        let signature = FuncType::new(self.linker.engine(), parameters, results);

        let registration = self.linker.func_new(
            module,
            name,
            signature,
            move |mut caller, arguments, results| {
                let host_state = &mut caller.data_mut().host_state;
                run_host_import(|| host_function(host_state, arguments, results))
            },
        );

        match registration {
            Ok(_) => Ok(()),
            Err(error) => Err(RuntimeError::Link(LinkError::ImportDefinition {
                module: module.to_string(),
                name: name.to_string(),
                source: error.into_boxed_dyn_error(),
            })),
        }
    }

    /// Checks every import of `module` against these definitions, once.
    ///
    /// The prepared module can then be instantiated repeatedly without repeating the check.
    pub(crate) fn prepare(
        &self,
        module: &CompiledModule,
    ) -> Result<PreparedModule<HostState>, RuntimeError> {
        // Wasmtime reports a cross-engine module as untyped text, so check the identity here.
        if !Engine::same(self.linker.engine(), module.wasmtime().engine()) {
            return Err(RuntimeError::EngineMismatch);
        }

        match self.linker.instantiate_pre(module.wasmtime()) {
            Ok(instance_pre) => Ok(PreparedModule::new(instance_pre)),
            Err(error) => Err(RuntimeError::Link(classify_import_failure(error))),
        }
    }
}

/// An unresolved import is typed by Wasmtime. With the engine already checked, the only other
/// import failure is a definition whose Wasm type doesn't match.
fn classify_import_failure(error: wasmtime::Error) -> LinkError {
    match error.downcast::<UnknownImportError>() {
        Ok(unknown_import) => LinkError::MissingImport {
            module: unknown_import.module().to_string(),
            name: unknown_import.name().to_string(),
        },
        Err(error) => LinkError::ImportMismatch {
            source: error.into_boxed_dyn_error(),
        },
    }
}
