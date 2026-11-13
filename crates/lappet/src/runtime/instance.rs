//! Instances: one store, one guest and one host state, owned together.
//!
//! An [`Instance`] owns everything mutable about one running guest. Nothing mutable is shared
//! between instances, and dropping an instance frees its guest and drops its host state once.
//!
//! [`Instance::call`] is the only way into guest code after instantiation. It owns quarantine:
//! the first fault marks the instance, and every later call is refused there without entering
//! the guest. V1 has no reset. A faulted instance can still close and drain its resources.

use super::error::{BoundaryFault, LinkError, RuntimeError};
use super::fault::{GuestEntry, classify_guest_failure};
use super::limits::InstanceLimits;
use super::resource::{
    DrainedResources, InstanceKind, ResourceError, ResourceTable, ResourceToken, Sendable,
    ThreadBound,
};
use super::staging::StagingTable;
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
    ) -> Result<&T, ResourceError> {
        self.store.data().resources.get(token)
    }

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
}

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
