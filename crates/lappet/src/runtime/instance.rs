//! Instances: one store, one guest and one host state, owned together.
//!
//! An [`Instance`] owns everything mutable about one running guest. Nothing mutable is shared
//! between instances, and dropping an instance frees its guest and drops its host state once.
//!
//! [`Instance::call`] is the guest entry after instantiation, and it owns quarantine. Context
//! moves into the store for one entry and back to the holder before the call returns.
//! [`Instance::call_scoped`] drives the native host ABI's call-scope helpers around one export.

use super::context::HostContext;
use super::error::{BoundaryFault, HostFault, LinkError, RuntimeError};
use super::fault::{GuestEntry, classify_guest_failure};
use super::limits::InstanceLimits;
use super::resource::{
    DrainedResources, InstanceKind, ResourceError, ResourceTable, ResourceToken, Sendable,
    ThreadBound,
};
use super::staging::StagingTable;
use std::any::TypeId;
use wasmtime::{InstancePre, Store, StoreLimits, WasmParams, WasmResults};

/// How long one guest entry may run, in epoch ticks the embedder advances.
///
/// Lappet starts no timer thread. The embedder advances the epoch through
/// [`super::RuntimeEngine::advance_epoch`], for example once per frame or from a watchdog. A
/// deadline bounds guest code only. It can't interrupt a blocking host call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Deadline {
    /// The guest may run until it returns.
    Unbounded,
    /// The guest is interrupted once the epoch has advanced this many ticks.
    AfterTicks(u64),
}

impl Deadline {
    fn ticks_beyond_current(self) -> u64 {
        match self {
            // Wasmtime saturates the sum, so this is the furthest deadline it can represent.
            Self::Unbounded => u64::MAX,
            Self::AfterTicks(ticks) => ticks,
        }
    }
}

/// Everything a store owns besides the guest itself.
pub(super) struct StoreData<HostState, Kind: InstanceKind = Sendable> {
    pub(super) host_state: HostState,
    pub(super) limits: InstanceLimits,
    store_limits: StoreLimits,
    pub(super) staging: StagingTable,
    pub(super) resources: ResourceTable<Kind::Object>,
    /// Empty between calls. [`Instance::with_context`] is the only writer.
    pub(super) context: Option<Box<dyn std::any::Any + Send>>,
}

/// A compiled module whose imports are already checked against one set of host imports.
pub(crate) struct PreparedModule<HostState: 'static, Kind: InstanceKind = Sendable> {
    instance_pre: InstancePre<StoreData<HostState, Kind>>,
}

impl<HostState: 'static, Kind: InstanceKind> PreparedModule<HostState, Kind> {
    pub(super) fn new(instance_pre: InstancePre<StoreData<HostState, Kind>>) -> Self {
        Self { instance_pre }
    }
}

impl<HostState, Kind> PreparedModule<HostState, Kind>
where
    HostState: super::resource::HostStateBound<Kind>,
    Kind: InstanceKind,
{
    /// Creates an isolated instance and runs its Wasm start function, if it has one.
    ///
    /// The limits and the deadline are in force before any guest code runs. On failure the
    /// host state is dropped with the store, and no instance exists to quarantine.
    pub(crate) fn instantiate(
        &self,
        host_state: HostState,
        limits: InstanceLimits,
        start_deadline: Deadline,
    ) -> Result<Instance<HostState, Kind>, RuntimeError> {
        let module = self.instance_pre.module();
        limits
            .check_module(module)
            .map_err(RuntimeError::limit_before_entry)?;

        let store_data = StoreData {
            host_state,
            limits,
            store_limits: limits.store_limits(),
            staging: StagingTable::new(),
            resources: ResourceTable::new(),
            context: None,
        };
        let mut store = Store::new(module.engine(), store_data);
        store.limiter(|store_data| &mut store_data.store_limits);

        // An epoch-enabled store starts with a deadline that has already passed.
        store.set_epoch_deadline(start_deadline.ticks_beyond_current());

        let instance = self
            .instance_pre
            .instantiate(&mut store)
            .map_err(|error| classify_guest_failure(error, GuestEntry::StartFunction))?;
        // A start function is an outermost entry. A ticket it leaves live is the same fault as
        // an export that returns with one live, and no instance is handed out.
        if store.data().staging.has_live_ticket() {
            return Err(RuntimeError::BoundaryFault(
                BoundaryFault::TicketLiveAtReturn,
            ));
        }
        Ok(Instance {
            store,
            instance,
            is_quarantined: false,
        })
    }
}

/// One running guest with its store and host state.
///
/// `Kind` is [`Sendable`] by default. A [`ThreadBound`] instance uses the same table and the
/// same calls, and the compiler stops it leaving its thread.
pub(crate) struct Instance<HostState: 'static, Kind: InstanceKind = Sendable> {
    store: Store<StoreData<HostState, Kind>>,
    instance: wasmtime::Instance,
    /// Set by the first fault and never cleared. Only [`Instance::call`] reads or writes it.
    is_quarantined: bool,
}

impl<HostState: 'static, Kind: InstanceKind> Instance<HostState, Kind> {
    /// Calls an exported function by its physical Wasm signature.
    ///
    /// Taking `&mut self` means no call can start while another is active. A missing export or
    /// a signature mismatch is a link error and leaves the instance usable, because no guest
    /// code ran. Any failure of the call itself quarantines the instance. That includes a
    /// native-side mistake Wasmtime rejects before entering the guest, such as a reference
    /// owned by another instance, which is reported as an internal host fault.
    pub(crate) fn call<Parameters, Results>(
        &mut self,
        export_name: &str,
        parameters: Parameters,
        deadline: Deadline,
    ) -> Result<Results, RuntimeError>
    where
        Parameters: WasmParams,
        Results: WasmResults,
    {
        if self.is_quarantined {
            return Err(RuntimeError::InstanceFaulted);
        }

        let Some(function) = self.instance.get_func(&mut self.store, export_name) else {
            return Err(RuntimeError::Link(LinkError::MissingExport {
                name: export_name.to_string(),
            }));
        };
        let typed_function =
            function
                .typed::<Parameters, Results>(&self.store)
                .map_err(|error| {
                    RuntimeError::Link(LinkError::ExportSignatureMismatch {
                        name: export_name.to_string(),
                        source: error.into_boxed_dyn_error(),
                    })
                })?;

        self.store
            .set_epoch_deadline(deadline.ticks_beyond_current());

        match typed_function.call(&mut self.store, parameters) {
            Ok(results) => {
                if self.has_live_staged_text() {
                    self.quarantine();
                    return Err(RuntimeError::BoundaryFault(
                        BoundaryFault::TicketLiveAtReturn,
                    ));
                }
                Ok(results)
            }
            Err(error) => {
                self.quarantine();
                Err(classify_guest_failure(error, GuestEntry::ExportedCall))
            }
        }
    }

    /// Calls an export with the holder's context moved into the instance for this call only.
    pub(crate) fn call_with_context<Parameters, Results>(
        &mut self,
        export_name: &str,
        parameters: Parameters,
        deadline: Deadline,
        context: &mut HostContext,
    ) -> Result<Results, RuntimeError>
    where
        Parameters: WasmParams,
        Results: WasmResults,
    {
        self.with_context(context, |instance| {
            instance.call(export_name, parameters, deadline)
        })
    }

    /// Checks `raw`, then passes that same value. A rejected token does not enter the guest,
    /// does not quarantine the instance and leaves the context holder untouched.
    pub(crate) fn call_with_token<Results>(
        &mut self,
        export_name: &str,
        raw: i64,
        type_id: TypeId,
        deadline: Deadline,
        context: &mut HostContext,
    ) -> Result<Results, RuntimeError>
    where
        Results: WasmResults,
    {
        if let Err(fault) = self.store.data().resources.check_argument(raw, type_id) {
            return Err(RuntimeError::ExportArgument(fault));
        }
        self.call_with_context(export_name, raw, deadline, context)
    }

    /// Opens a scope, reserves the argument string and result frame, calls the export once with
    /// the address and length carriers, copies the returned span and closes. A fault skips close.
    pub(crate) fn call_scoped(
        &mut self,
        export_name: &str,
        argument: &[u8],
        deadline: Deadline,
        context: &mut HostContext,
    ) -> Result<ScopedResult, RuntimeError> {
        self.with_context(context, |instance| {
            instance.drive_scope(export_name, argument, deadline)
        })
    }

    /// Whether a context value is still in the instance. It is false between calls.
    pub(crate) fn has_context(&self) -> bool {
        self.store.data().context.is_some()
    }

    fn with_context<Results>(
        &mut self,
        context: &mut HostContext,
        body: impl FnOnce(&mut Self) -> Result<Results, RuntimeError>,
    ) -> Result<Results, RuntimeError> {
        if self.has_context() {
            self.quarantine();
            return Err(RuntimeError::HostFault(HostFault::Internal {
                source: "context was left in the instance between calls".into(),
            }));
        }
        self.store.data_mut().context = context.take();
        let result = body(self);
        context.restore(self.store.data_mut().context.take());
        result
    }

    fn drive_scope(
        &mut self,
        export_name: &str,
        argument: &[u8],
        deadline: Deadline,
    ) -> Result<ScopedResult, RuntimeError> {
        let Ok(argument_size) = i32::try_from(argument.len()) else {
            return Err(RuntimeError::HostFault(HostFault::Internal {
                source: "an export argument string does not fit an i32 length".into(),
            }));
        };
        self.call::<(), ()>(SCOPE_OPEN, (), deadline)?;
        match self.drive_open_scope(export_name, argument, argument_size, deadline) {
            Ok(copied) => match self.call::<(), ()>(SCOPE_CLOSE, (), deadline) {
                Ok(()) => Ok(ScopedResult { copied }),
                Err(error) => Err(self.fault_open_scope(error)),
            },
            Err(error) => Err(self.fault_open_scope(error)),
        }
    }

    /// A scope is open. Any error here must not call scope close.
    fn drive_open_scope(
        &mut self,
        export_name: &str,
        argument: &[u8],
        argument_size: i32,
        deadline: Deadline,
    ) -> Result<String, RuntimeError> {
        let argument_address = if argument.is_empty() {
            0
        } else {
            let address = self.reserve(argument_size, FRAME_ALIGN, deadline)?;
            self.write_memory(address, argument)?;
            address
        };
        let frame = self.reserve(FRAME_SIZE, FRAME_ALIGN, deadline)?;
        self.write_memory(frame, &[0; FRAME_SIZE as usize])?;
        let status = self.call::<(i32, i32, i32), i32>(
            export_name,
            (argument_address, argument_size, frame),
            deadline,
        )?;
        if status != 0 {
            return Err(RuntimeError::BoundaryFault(
                BoundaryFault::NoncanonicalStatus,
            ));
        }
        self.read_span(frame)
    }

    fn fault_open_scope(&mut self, error: RuntimeError) -> RuntimeError {
        if !self.is_quarantined {
            self.quarantine();
        }
        error
    }

    fn reserve(&mut self, size: i32, align: i32, deadline: Deadline) -> Result<i32, RuntimeError> {
        let address = self.call::<(i32, i32), i32>(SCOPE_RESERVE, (size, align), deadline)?;
        if address < 0 {
            return Err(RuntimeError::BoundaryFault(
                BoundaryFault::DestinationOutOfBounds,
            ));
        }
        Ok(address)
    }

    fn write_memory(&mut self, address: i32, bytes: &[u8]) -> Result<(), RuntimeError> {
        let memory = self.scope_memory()?;
        let offset = usize::try_from(address)
            .map_err(|_| RuntimeError::BoundaryFault(BoundaryFault::DestinationOutOfBounds))?;
        memory
            .write(&mut self.store, offset, bytes)
            .map_err(|_| RuntimeError::BoundaryFault(BoundaryFault::DestinationOutOfBounds))
    }

    fn read_memory(&mut self, offset: usize, bytes: &mut [u8]) -> Result<(), RuntimeError> {
        let memory = self.scope_memory()?;
        memory
            .read(&mut self.store, offset, bytes)
            .map_err(|_| RuntimeError::BoundaryFault(BoundaryFault::DestinationOutOfBounds))
    }

    fn scope_memory(&mut self) -> Result<wasmtime::Memory, RuntimeError> {
        self.instance
            .get_memory(&mut self.store, SCOPE_MEMORY)
            .ok_or_else(|| {
                RuntimeError::HostFault(HostFault::Internal {
                    source: "the module exports no memory named `memory`".into(),
                })
            })
    }

    fn read_span(&mut self, frame: i32) -> Result<String, RuntimeError> {
        let mut header = [0u8; FRAME_SIZE as usize];
        let frame_offset = usize::try_from(frame)
            .map_err(|_| RuntimeError::BoundaryFault(BoundaryFault::DestinationOutOfBounds))?;
        self.read_memory(frame_offset, &mut header)?;
        let address = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if length == 0 {
            if address != 0 {
                return Err(RuntimeError::BoundaryFault(
                    BoundaryFault::NoncanonicalStatus,
                ));
            }
            return Ok(String::new());
        }
        if length > 2_147_483_647 {
            return Err(RuntimeError::BoundaryFault(
                BoundaryFault::NoncanonicalStatus,
            ));
        }
        let start = usize::try_from(address)
            .map_err(|_| RuntimeError::BoundaryFault(BoundaryFault::DestinationOutOfBounds))?;
        let Some(end) = start.checked_add(length as usize) else {
            return Err(RuntimeError::BoundaryFault(
                BoundaryFault::DestinationOutOfBounds,
            ));
        };
        let memory_size = self.scope_memory()?.data_size(&self.store);
        if end > memory_size {
            return Err(RuntimeError::BoundaryFault(
                BoundaryFault::DestinationOutOfBounds,
            ));
        }
        let mut bytes = vec![0u8; length as usize];
        self.read_memory(start, &mut bytes)?;
        match std::str::from_utf8(&bytes) {
            Ok(text) => Ok(text.to_string()),
            Err(_) => Err(RuntimeError::BoundaryFault(
                BoundaryFault::NoncanonicalStatus,
            )),
        }
    }

    /// The instance's host state. It stays readable after a fault, so the embedder can inspect
    /// and drain what a quarantined instance still owns.
    pub(crate) fn host_state(&self) -> &HostState {
        &self.store.data().host_state
    }

    pub(crate) fn host_state_mut(&mut self) -> &mut HostState {
        &mut self.store.data_mut().host_state
    }

    /// Stages text for the guest. The empty slice returns ticket zero and stores nothing.
    pub(crate) fn stage_text(&mut self, bytes: &[u8]) -> Result<u64, super::staging::StagingError> {
        let limits = self.store.data().limits;
        self.store.data_mut().staging.stage(bytes, &limits)
    }

    pub(crate) fn has_live_staged_text(&self) -> bool {
        self.store.data().staging.has_live_ticket()
    }

    /// A fault ends guest calls. The staging table is cleared. Resources stay, so the embedder
    /// can still close and drain them.
    fn quarantine(&mut self) {
        self.is_quarantined = true;
        self.store.data_mut().staging.clear();
    }

    /// Resolves a token to its value. The type is checked before the reference is returned.
    pub(crate) fn resource<T: 'static>(
        &self,
        token: ResourceToken<T>,
    ) -> Result<super::resource::Shared<'_, T>, ResourceError> {
        self.store.data().resources.get(token)
    }

    pub(crate) fn resource_mut<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<super::resource::Exclusive<'_, T>, ResourceError> {
        self.store.data_mut().resources.get_mut(token)
    }

    /// Removes the entry and returns the value. A faulted instance can still close.
    pub(crate) fn close_resource<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<T, ResourceError> {
        self.store.data_mut().resources.close(token)
    }

    /// Returns every remaining value in creation order and leaves the table empty.
    pub(crate) fn drain_resources(&mut self) -> DrainedResources {
        self.store.data_mut().resources.drain()
    }
}

/// The string copied out of the scope before it was closed.
pub(crate) struct ScopedResult {
    pub(crate) copied: String,
}

const SCOPE_OPEN: &str = "moth_abi_v1_scope_open";
const SCOPE_RESERVE: &str = "moth_abi_v1_scope_reserve";
const SCOPE_CLOSE: &str = "moth_abi_v1_scope_close";
const SCOPE_MEMORY: &str = "memory";
/// An `AbiSpan`: address then UTF-8 byte length.
const FRAME_SIZE: i32 = 8;
const FRAME_ALIGN: i32 = 4;

impl<HostState: 'static> Instance<HostState, Sendable> {
    /// Inserts a `Send` value and returns its token. The table owns the value.
    pub(crate) fn insert_resource<T: Send + 'static>(
        &mut self,
        value: T,
    ) -> Result<ResourceToken<T>, ResourceError> {
        let limits = self.store.data().limits;
        self.store.data_mut().resources.insert(value, &limits)
    }
}

impl<HostState: 'static> Instance<HostState, ThreadBound> {
    /// Inserts any `'static` value. The instance cannot move to another thread.
    pub(crate) fn insert_resource<T: 'static>(
        &mut self,
        value: T,
    ) -> Result<ResourceToken<T>, ResourceError> {
        let limits = self.store.data().limits;
        self.store.data_mut().resources.insert(value, &limits)
    }
}

impl<HostState: 'static, Kind: InstanceKind> Drop for Instance<HostState, Kind> {
    fn drop(&mut self) {
        // Teardown uses the same clear as a fault, so a live ticket can't outlive the instance
        // even if a later drop observer still sees the store. Resources drop with the store, in
        // creation order, from the table's own drop.
        self.store.data_mut().staging.clear();
    }
}
