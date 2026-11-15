//! Instances: one store, one guest and one host state, owned together.
//!
//! An [`Instance`] owns everything mutable about one running guest. Nothing mutable is shared
//! between instances, and dropping an instance frees its guest and drops its host state once.
//!
//! [`Instance::call`] is the guest entry after instantiation, and it owns quarantine. Context
//! moves into the store for one entry and back to the holder before the call returns.
//! [`Instance::call_scoped`] drives the native host ABI's call-scope helpers around one export.
//!
//! `HostState` is the instance-owned slot the crate-private import closures use. It is not a
//! third ABI role. Resources and context are the two roles a host function receives. Adapters
//! will see those through one call view and will not grow a parallel state generic.

use super::context::HostContext;
use super::error::{BoundaryFault, HostFault, LinkError, RuntimeError};
use super::fault::{GuestEntry, classify_guest_failure};
use super::kind::{HostStateBound, InstanceKind, Sendable, ThreadBound};
use super::limits::InstanceLimits;
use super::resource::{DrainedResources, ResourceError, ResourceTable, ResourceToken, Shared};
use super::staging::{StagingError, StagingTable};
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
///
/// `host_state` is the pre-adapter slot described above. Context follows the instance kind:
/// sendable context is `Send`, and thread-bound context is any `'static` value.
pub(super) struct StoreData<HostState, Kind: InstanceKind = Sendable> {
    pub(super) host_state: HostState,
    pub(super) limits: InstanceLimits,
    store_limits: StoreLimits,
    pub(super) staging: StagingTable,
    pub(super) resources: ResourceTable<Kind::Object>,
    /// Empty between calls. [`Instance::with_context`] is the only writer.
    pub(super) context: Option<Box<Kind::Context>>,
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
    HostState: HostStateBound<Kind>,
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
            #[cfg(test)]
            deadline_installs: 0,
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
    /// Set by the first fault and never cleared. [`Instance::call`] and the scope driver write it.
    is_quarantined: bool,
    #[cfg(test)]
    deadline_installs: u32,
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
        self.install_deadline(deadline);
        self.invoke(export_name, parameters)
    }

    /// Enters an export without installing a new deadline. The scope driver installs one
    /// deadline for the whole export, including its helpers.
    fn invoke<Parameters, Results>(
        &mut self,
        export_name: &str,
        parameters: Parameters,
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

    fn install_deadline(&mut self, deadline: Deadline) {
        self.store
            .set_epoch_deadline(deadline.ticks_beyond_current());
        #[cfg(test)]
        {
            self.deadline_installs += 1;
        }
    }

    /// Calls an export with the holder's context moved into the instance for this call only.
    pub(crate) fn call_with_context<Parameters, Results>(
        &mut self,
        export_name: &str,
        parameters: Parameters,
        deadline: Deadline,
        context: &mut HostContext<Kind>,
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
        context: &mut HostContext<Kind>,
    ) -> Result<Results, RuntimeError>
    where
        Results: WasmResults,
    {
        self.store
            .data()
            .resources
            .check_argument(raw, type_id)
            .map_err(RuntimeError::ExportArgument)?;
        self.call_with_context(export_name, raw, deadline, context)
    }

    /// Opens a scope, reserves the argument string and result frame, calls the export once with
    /// the address and length carriers, copies the returned span and closes. A fault skips close.
    pub(crate) fn call_scoped(
        &mut self,
        export_name: &str,
        argument: &[u8],
        deadline: Deadline,
        context: &mut HostContext<Kind>,
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
        context: &mut HostContext<Kind>,
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
        // One budget covers open, reserve, the export and close. Resetting it on each
        // entry would multiply the deadline by the number of guest entries.
        self.install_deadline(deadline);
        self.invoke::<(), ()>(SCOPE_OPEN, ())?;

        // A fault inside the open scope skips close. A declared error is not a fault, so the
        // scope still closes before either outcome is reported.
        let body = match self.drive_open_scope(export_name, argument, argument_size) {
            Ok(body) => body,
            Err(error) => return Err(self.fault_open_scope(error)),
        };
        if let Err(error) = self.invoke::<(), ()>(SCOPE_CLOSE, ()) {
            return Err(self.fault_open_scope(error));
        }
        match body {
            ScopeBody::Copied(copied) => Ok(ScopedResult { copied }),
            ScopeBody::DeclaredError => Err(RuntimeError::DeclaredError),
        }
    }

    /// A scope is open. Any error here must not call scope close. Status `1` is a declared
    /// error, not a fault, so the caller still closes.
    fn drive_open_scope(
        &mut self,
        export_name: &str,
        argument: &[u8],
        argument_size: i32,
    ) -> Result<ScopeBody, RuntimeError> {
        let argument_address = if argument.is_empty() {
            0
        } else {
            let address = self.reserve(argument_size, FRAME_ALIGN)?;
            self.write_memory(address, argument)?;
            address
        };
        let frame = self.reserve(FRAME_SIZE, FRAME_ALIGN)?;
        self.write_memory(frame, &[0; FRAME_SIZE as usize])?;
        let status = self.invoke::<(i32, i32, i32), i32>(
            export_name,
            (argument_address, argument_size, frame),
        )?;
        match status {
            0 => self.read_span(frame).map(ScopeBody::Copied),
            // The fixture frame is the success span. Decoding `AbiGuestError` arrives with
            // the adapters. Status `1` is still the declared-error edge, not a bad status.
            1 => Ok(ScopeBody::DeclaredError),
            _ => Err(RuntimeError::BoundaryFault(
                BoundaryFault::NoncanonicalStatus,
            )),
        }
    }

    /// Any failure while a scope is open is a fault, including a link error that left the
    /// guest unentered, because the open scope can no longer be closed.
    fn fault_open_scope(&mut self, error: RuntimeError) -> RuntimeError {
        self.quarantine();
        error
    }

    fn reserve(&mut self, size: i32, align: i32) -> Result<i32, RuntimeError> {
        // Failure traps inside the helper. A returned value is an address, including one at
        // or above 2 GiB, whose `i32` carrier is negative.
        self.invoke::<(i32, i32), i32>(SCOPE_RESERVE, (size, align))
    }

    fn write_memory(&mut self, address: i32, bytes: &[u8]) -> Result<(), RuntimeError> {
        let memory = self.scope_memory()?;
        memory
            .write(&mut self.store, guest_offset(address), bytes)
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
            .get_memory(&mut self.store, GUEST_MEMORY_EXPORT)
            .ok_or_else(|| {
                RuntimeError::HostFault(HostFault::Internal {
                    source: "the module exports no memory named `memory`".into(),
                })
            })
    }

    fn read_span(&mut self, frame: i32) -> Result<String, RuntimeError> {
        let mut header = [0u8; FRAME_SIZE as usize];
        self.read_memory(guest_offset(frame), &mut header)?;
        let [a0, a1, a2, a3, l0, l1, l2, l3] = header;
        let address = u32::from_le_bytes([a0, a1, a2, a3]);
        let length = u32::from_le_bytes([l0, l1, l2, l3]);

        let noncanonical = || RuntimeError::BoundaryFault(BoundaryFault::NoncanonicalStatus);
        let out_of_bounds = || RuntimeError::BoundaryFault(BoundaryFault::DestinationOutOfBounds);

        // The empty string is the zero span, and a length never exceeds the `i32` carrier.
        if length == 0 {
            return if address == 0 {
                Ok(String::new())
            } else {
                Err(noncanonical())
            };
        }
        if length > i32::MAX.cast_unsigned() {
            return Err(noncanonical());
        }

        // Check the range before allocating, so a wild length can't size the buffer.
        let start = usize::try_from(address).map_err(|_| out_of_bounds())?;
        let length = usize::try_from(length).map_err(|_| out_of_bounds())?;
        let memory_size = self.scope_memory()?.data_size(&self.store);
        let is_in_bounds = start
            .checked_add(length)
            .is_some_and(|end| end <= memory_size);
        if !is_in_bounds {
            return Err(out_of_bounds());
        }

        let mut bytes = vec![0u8; length];
        self.read_memory(start, &mut bytes)?;
        String::from_utf8(bytes).map_err(|_| noncanonical())
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
    pub(crate) fn stage_text(&mut self, bytes: &[u8]) -> Result<u64, StagingError> {
        let store_data = self.store.data_mut();
        store_data.staging.stage(bytes, &store_data.limits)
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
    ) -> Result<Shared<'_, T>, ResourceError> {
        self.store.data().resources.get(token)
    }

    /// Resolves a token to its value for writing. `&mut self` proves no guest call is running.
    pub(crate) fn resource_mut<T: 'static>(
        &mut self,
        token: ResourceToken<T>,
    ) -> Result<&mut T, ResourceError> {
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

    #[cfg(test)]
    pub(crate) fn deadline_installs(&self) -> u32 {
        self.deadline_installs
    }
}

/// The string copied out of the scope before it was closed.
pub(crate) struct ScopedResult {
    pub(crate) copied: String,
}

enum ScopeBody {
    Copied(String),
    DeclaredError,
}

const SCOPE_OPEN: &str = "moth_abi_v1_scope_open";
const SCOPE_RESERVE: &str = "moth_abi_v1_scope_reserve";
const SCOPE_CLOSE: &str = "moth_abi_v1_scope_close";
/// Guest fixtures export their one linear memory under this name. Compiled Moth has no export
/// name for it yet, so the take import and the scope driver share the fixture convention.
pub(super) const GUEST_MEMORY_EXPORT: &str = "memory";
/// An `AbiSpan`: address then UTF-8 byte length.
const FRAME_SIZE: i32 = 8;
const FRAME_ALIGN: i32 = 4;

/// The `i32` carrier's bits are a Wasm32 address, including addresses at or above 2 GiB.
pub(crate) fn guest_offset(address: i32) -> usize {
    address as u32 as usize
}

impl<HostState: 'static> Instance<HostState, Sendable> {
    /// Inserts a `Send` value and returns its token. The table owns the value.
    pub(crate) fn insert_resource<T: Send + 'static>(
        &mut self,
        value: T,
    ) -> Result<ResourceToken<T>, ResourceError> {
        let store_data = self.store.data_mut();
        store_data.resources.insert(value, &store_data.limits)
    }
}

impl<HostState: 'static> Instance<HostState, ThreadBound> {
    /// Inserts any `'static` value. The instance cannot move to another thread.
    pub(crate) fn insert_resource<T: 'static>(
        &mut self,
        value: T,
    ) -> Result<ResourceToken<T>, ResourceError> {
        let store_data = self.store.data_mut();
        store_data.resources.insert(value, &store_data.limits)
    }
}
