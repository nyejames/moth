//! Raw host import registration, checked once against a compiled module.
//!
//! This is the crate-private mechanism the host adapters will build on. An import is named and
//! typed by its physical Wasm signature here. Canonical identities, descriptors and value
//! conversion belong to the adapters.
//!
//! A host import receives its instance's host state, the staging table, the per-call context
//! and the call's Wasm values. It gets no handle that could call an export, so it can't
//! re-enter the guest. The reserved take import is the one exception that writes guest memory,
//! and it writes only.

use super::context::CallContext;
use super::engine::RuntimeEngine;
use super::error::{BoundaryFault, ErrorSource, LinkError, RuntimeError};
use super::fault::run_host_import;
use super::instance::{PreparedModule, StoreData};
use super::limits::InstanceLimits;
use super::module::CompiledModule;
use super::resource::{InstanceKind, ResourceTable};
use super::staging::{StagingError, StagingTable};
use wasmtime::{Caller, Engine, Extern, FuncType, Linker, UnknownImportError, Val, ValType};

/// The host imports one kind of instance provides, before they're checked against a module.
///
/// Registration is store-independent, so one set of imports serves every instance prepared
/// from it. Mutable state lives in each instance's `HostState`, never in the import closures.
pub(crate) struct HostImports<HostState: 'static, Kind: InstanceKind = super::resource::Sendable> {
    linker: Linker<StoreData<HostState, Kind>>,
}

impl<HostState: 'static, Kind: InstanceKind> HostImports<HostState, Kind> {
    pub(crate) fn new(engine: &RuntimeEngine) -> Self {
        // A new linker rejects a second definition of one name and defines nothing by itself.
        // An import the host didn't register is therefore always a link error, never a stub.
        let mut linker = Linker::new(engine.wasmtime());
        install_staged_text_take(&mut linker);
        Self { linker }
    }

    /// Registers one host function under its physical Wasm signature.
    ///
    /// The function fills `results` in order. A failure it returns becomes a host fault that
    /// ends the guest call, and so does a panic.
    ///
    /// The signature uses the numeric Wasm carriers. A reference type built for another engine
    /// is a caller bug and panics inside Wasmtime.
    pub(crate) fn define(
        &mut self,
        module: &str,
        name: &str,
        parameters: impl IntoIterator<Item = ValType>,
        results: impl IntoIterator<Item = ValType>,
        host_function: impl Fn(
            &mut HostState,
            &mut StagingTable,
            &InstanceLimits,
            &ResourceTable<Kind::Object>,
            &mut CallContext<'_>,
            &[Val],
            &mut [Val],
        ) -> Result<(), ErrorSource>
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
                if caller.data().staging.has_live_ticket() {
                    return Err(wasmtime::Error::new(BoundaryFault::TicketLiveAtImport));
                }
                let data = caller.data_mut();
                let mut context = CallContext::new(&mut data.context);
                run_host_import(|| {
                    host_function(
                        &mut data.host_state,
                        &mut data.staging,
                        &data.limits,
                        &data.resources,
                        &mut context,
                        arguments,
                        results,
                    )
                })
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
    ) -> Result<PreparedModule<HostState, Kind>, RuntimeError> {
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

/// The ABI name, defined once. The module and the function are the two halves of
/// `moth-abi-v1.staged_text_take`.
const STAGED_TEXT_MODULE: &str = "moth-abi-v1";
const STAGED_TEXT_TAKE: &str = "staged_text_take";

/// Guest fixtures export their one linear memory under this name. Compiled Moth has no export
/// name for it yet, so the take import uses the fixture convention.
const GUEST_MEMORY_EXPORT: &str = "memory";

fn install_staged_text_take<HostState: 'static, Kind: InstanceKind>(
    linker: &mut Linker<StoreData<HostState, Kind>>,
) {
    let signature = FuncType::new(
        linker.engine(),
        [ValType::I64, ValType::I32, ValType::I32],
        [],
    );
    linker
        .func_new(
            STAGED_TEXT_MODULE,
            STAGED_TEXT_TAKE,
            signature,
            |mut caller, arguments, _results| {
                let ticket = arguments[0]
                    .i64()
                    .expect("the take import takes an i64 ticket")
                    as u64;
                let destination = arguments[1]
                    .i32()
                    .expect("the take import takes an i32 destination");
                let length = arguments[2]
                    .i32()
                    .expect("the take import takes an i32 length");
                copy_staged_text(&mut caller, ticket, destination, length)
            },
        )
        .expect("a fresh linker accepts the reserved take import");
}

fn copy_staged_text<HostState, Kind: InstanceKind>(
    caller: &mut Caller<'_, StoreData<HostState, Kind>>,
    ticket: u64,
    destination: i32,
    length: i32,
) -> wasmtime::Result<()> {
    let bytes = caller
        .data()
        .staging
        .staged_bytes(ticket)
        .map_err(boundary_from_staging)?;
    let Ok(length) = usize::try_from(length) else {
        return Err(wasmtime::Error::new(BoundaryFault::WrongLength));
    };
    if bytes.len() != length {
        return Err(wasmtime::Error::new(BoundaryFault::WrongLength));
    }
    // The i32 carrier's bits are a Wasm32 address, including addresses at or above 2 GiB.
    let destination = destination as u32 as usize;

    let Some(Extern::Memory(memory)) = caller.get_export(GUEST_MEMORY_EXPORT) else {
        return Err(wasmtime::Error::new(BoundaryFault::DestinationOutOfBounds));
    };
    memory
        .write(&mut *caller, destination, &bytes)
        .map_err(|_| wasmtime::Error::new(BoundaryFault::DestinationOutOfBounds))?;

    caller
        .data_mut()
        .staging
        .take(ticket, length)
        .map_err(boundary_from_staging)?;
    Ok(())
}

fn boundary_from_staging(error: StagingError) -> wasmtime::Error {
    let fault = match error {
        StagingError::UnknownTicket => BoundaryFault::UnknownTicket,
        StagingError::WrongLength => BoundaryFault::WrongLength,
        // A take never reports a stage-time limit. Those are refused before a ticket exists.
        StagingError::EntryTooLarge
        | StagingError::LiveBytesExceeded
        | StagingError::TicketsExhausted => BoundaryFault::UnknownTicket,
    };
    wasmtime::Error::new(fault)
}
