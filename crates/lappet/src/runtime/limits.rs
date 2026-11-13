//! Per-instance resource limits.
//!
//! Limits are enforced by Wasmtime's store limiter, installed before any guest code runs. The
//! module's declared requirements are also checked up front, because Wasmtime reports a limiter
//! denial at instantiation as untyped text and the runtime classifies by typed facts only.
//!
//! These limits bound guest allocations. They don't bound Rust host allocations, native
//! libraries or a blocking host call.

use super::error::LimitFault;
use wasmtime::{Module, StoreLimits, StoreLimitsBuilder};

/// One Moth module per instance in V1, with one linear memory and one table.
const INSTANCES_PER_STORE: usize = 1;
const MEMORIES_PER_INSTANCE: u32 = 1;
const TABLES_PER_INSTANCE: u32 = 1;

/// The Wasm page size. The engine switches custom page sizes off, so every memory uses it.
const WASM_PAGE_BYTES: u64 = 65_536;

/// The ceilings one instance's guest allocations may reach.
///
/// Growth past a ceiling is denied without a fault: `memory.grow` and `table.grow` return `-1`
/// to the guest, which decides what that means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceLimits {
    /// Largest size, in bytes, the instance's linear memory may reach.
    pub max_memory_bytes: usize,
    /// Largest element count the instance's table may reach.
    pub max_table_elements: usize,
}

impl Default for InstanceLimits {
    /// The provisional V1 defaults: 256 MiB of linear memory and 65,536 table elements.
    fn default() -> Self {
        Self {
            max_memory_bytes: 256 * 1024 * 1024,
            max_table_elements: 65_536,
        }
    }
}

impl InstanceLimits {
    /// Rejects a module whose declared memories and tables can't fit, before instantiation.
    pub(super) fn check_module(&self, module: &Module) -> Result<(), LimitFault> {
        let required = module.resources_required();

        if required.num_memories > MEMORIES_PER_INSTANCE {
            return Err(LimitFault::MemoryCount);
        }
        if required.num_tables > TABLES_PER_INSTANCE {
            return Err(LimitFault::TableCount);
        }

        let initial_memory_bytes = required
            .max_initial_memory_size
            .unwrap_or(0)
            .saturating_mul(WASM_PAGE_BYTES);
        if exceeds(initial_memory_bytes, self.max_memory_bytes) {
            return Err(LimitFault::MemoryCeiling);
        }

        let initial_table_elements = required.max_initial_table_size.unwrap_or(0);
        if exceeds(initial_table_elements, self.max_table_elements) {
            return Err(LimitFault::TableCeiling);
        }

        Ok(())
    }

    /// The limiter Wasmtime consults for every allocation and growth in one store.
    pub(super) fn store_limits(&self) -> StoreLimits {
        // Every count is set, so nothing falls back to Wasmtime's 10,000-object defaults.
        StoreLimitsBuilder::new()
            .memory_size(self.max_memory_bytes)
            .table_elements(self.max_table_elements)
            .instances(INSTANCES_PER_STORE)
            .memories(MEMORIES_PER_INSTANCE as usize)
            .tables(TABLES_PER_INSTANCE as usize)
            .trap_on_grow_failure(false)
            .build()
    }
}

/// Whether a module requirement is above a host-sized ceiling. A requirement the host can't
/// even represent is above every ceiling.
fn exceeds(required: u64, ceiling: usize) -> bool {
    usize::try_from(required).map_or(true, |required| required > ceiling)
}
