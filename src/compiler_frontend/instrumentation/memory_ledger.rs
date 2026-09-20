//! Feature-gated memory ownership ledger for the data-layout probe.
//!
//! The ledger records compiler-owned storage, not allocator events. Every live owner is keyed by
//! the address of its allocation, and owner `Drop` hooks retire that live key before an allocator
//! address can be reused. Cumulative totals remain after retirement so preparation failures and
//! diagnosed/dropped artefacts stay visible. The ledger is reset at the command-scoped frontend
//! boundary and consumed once at the render boundary.

#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathTable};
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::symbols::string_interning::FrozenStringTable;
#[cfg(feature = "data_layout_memory_probe")]
use crate::compiler_frontend::tokenizer::tokens::SourceTokens;

/// Stable machine-readable memory fields emitted by `data_layout_memory_probe`.
#[cfg(feature = "data_layout_memory_probe")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemoryLedgerSnapshot {
    pub(crate) source_tokens_owners: usize,
    pub(crate) source_tokens_shape_bytes: usize,
    pub(crate) source_tokens_shape_capacity_bytes: usize,
    pub(crate) source_tokens_span_bytes: usize,
    pub(crate) source_tokens_span_capacity_bytes: usize,
    pub(crate) source_tokens_numeric_store_bytes: usize,
    pub(crate) source_tokens_numeric_store_capacity_bytes: usize,
    pub(crate) source_tokens_path_store_bytes: usize,
    pub(crate) source_tokens_path_store_capacity_bytes: usize,
    pub(crate) source_tokens_sequence_store_bytes: usize,
    pub(crate) source_tokens_sequence_store_capacity_bytes: usize,
    /// Cumulative sum of builder shape/span backing capacities sampled at `finish`.
    /// This is construction pressure, not simultaneous live storage.
    pub(crate) transient_construction_buffer_bytes: usize,
    /// Largest individual builder shape/span capacity sample observed at `finish`.
    /// It is a high-water observation, not a claim that all builders coexist.
    pub(crate) transient_construction_peak_bytes: usize,
    pub(crate) cutover_adapter_bytes: usize,
    pub(crate) donor_identity_string_tables: usize,
    pub(crate) donor_identity_string_storage_bytes: usize,
    pub(crate) donor_identity_path_tables: usize,
    pub(crate) donor_identity_path_storage_bytes: usize,
    pub(crate) generic_source_tokens_owners: usize,
    /// Cumulative bytes for every distinct generic source owner observed by the command.
    pub(crate) generic_source_tokens_bytes: usize,
    /// Distinct generic source-owner bytes whose tracked body references are live at the snapshot.
    pub(crate) generic_source_tokens_live_bytes: usize,
    /// High-water distinct generic source-owner bytes observed at a ledger event.
    pub(crate) generic_source_tokens_peak_bytes: usize,
    /// Number of requester remap tables constructed during this command.
    pub(crate) requester_remap_count: usize,
    pub(crate) requester_remap_used_bytes: usize,
    pub(crate) requester_remap_capacity_bytes: usize,
    /// True when a bounded ledger map could not retain every distinct owner identity.
    pub(crate) observation_incomplete: bool,
}

#[cfg(feature = "data_layout_memory_probe")]
mod enabled {
    use super::{
        FrozenStringTable, MemoryLedgerSnapshot, PathIdRemap, PathSyntaxTable, PathTable,
        SourceTokens,
    };
    use crate::compiler_frontend::tokenizer::tokens::SourceTokensMemoryLedgerMetrics;
    use rustc_hash::FxHashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    // These are deliberately bounded warm-up capacities. The probe calls `prepare` before taking
    // its allocator baseline; recording never grows a map past its prepared capacity. An
    // unexpectedly larger fixture is marked incomplete and safely under-counted rather than
    // contaminating allocator evidence with ledger bookkeeping allocations.
    const SOURCE_TOKEN_OWNER_CAPACITY: usize = 16_384;
    const SOURCE_PATH_TABLE_CAPACITY: usize = 16_384;
    const GENERIC_SOURCE_TOKEN_CAPACITY: usize = 16_384;
    const DONOR_IDENTITY_TABLE_CAPACITY: usize = 8_192;

    #[derive(Clone, Copy, Debug)]
    struct StoreBytes {
        bytes: usize,
        capacity_bytes: usize,
    }

    #[derive(Clone, Copy, Debug)]
    struct SourceTokenOwner {
        shape_bytes: usize,
        shape_capacity_bytes: usize,
        span_bytes: usize,
        span_capacity_bytes: usize,
        numeric_store_bytes: usize,
        numeric_store_capacity_bytes: usize,
        sequence_store_bytes: usize,
        sequence_store_capacity_bytes: usize,
        path_table: Option<(usize, StoreBytes)>,
    }

    #[derive(Clone, Copy, Debug)]
    struct GenericSourceTokenOwner {
        bytes: usize,
        live_references: usize,
    }

    #[derive(Clone, Copy, Debug, Default)]
    struct SourceTokenTotals {
        owners: usize,
        shape_bytes: usize,
        shape_capacity_bytes: usize,
        span_bytes: usize,
        span_capacity_bytes: usize,
        numeric_store_bytes: usize,
        numeric_store_capacity_bytes: usize,
        path_store_bytes: usize,
        path_store_capacity_bytes: usize,
        sequence_store_bytes: usize,
        sequence_store_capacity_bytes: usize,
    }

    #[derive(Debug, Default)]
    struct MemoryLedger {
        // Live identity maps are retired by owner Drop hooks. Their values are only bookkeeping;
        // compiler-owned Arcs are never retained by this ledger.
        source_tokens: FxHashMap<usize, SourceTokenOwner>,
        source_path_tables: FxHashMap<usize, StoreBytes>,
        generic_source_tokens: FxHashMap<usize, GenericSourceTokenOwner>,
        donor_identity_strings: FxHashMap<usize, usize>,
        donor_identity_paths: FxHashMap<usize, usize>,
        source_totals: SourceTokenTotals,
        generic_source_tokens_owners: usize,
        generic_source_tokens_bytes: usize,
        generic_source_tokens_peak_bytes: usize,
        donor_identity_string_tables: usize,
        donor_identity_string_storage_bytes: usize,
        donor_identity_path_tables: usize,
        donor_identity_path_storage_bytes: usize,
        transient_construction_buffer_bytes: usize,
        transient_construction_peak_bytes: usize,
        requester_remap_count: usize,
        requester_remap_used_bytes: usize,
        requester_remap_capacity_bytes: usize,
        observation_incomplete: bool,
        prepared: bool,
        last_snapshot: Option<MemoryLedgerSnapshot>,
    }

    fn ledger() -> &'static Mutex<MemoryLedger> {
        static LEDGER: LazyLock<Mutex<MemoryLedger>> =
            LazyLock::new(|| Mutex::new(MemoryLedger::default()));
        &LEDGER
    }

    fn with_ledger<R>(f: impl FnOnce(&mut MemoryLedger) -> R) -> R {
        let mut guard = ledger()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    /// Allocate all ledger buckets before the probe's allocator baseline.
    ///
    /// This hook is intentionally owned by the existing probe lane. `reset` below clears entries
    /// without releasing these buckets, and recording refuses to grow a full bucket. Consequently
    /// ordinary fixtures do not add ledger allocations to live/peak/after-report measurements.
    pub(crate) fn prepare() {
        with_ledger(|ledger| {
            ledger
                .source_tokens
                .reserve(SOURCE_TOKEN_OWNER_CAPACITY);
            ledger
                .source_path_tables
                .reserve(SOURCE_PATH_TABLE_CAPACITY);
            ledger
                .generic_source_tokens
                .reserve(GENERIC_SOURCE_TOKEN_CAPACITY);
            ledger
                .donor_identity_strings
                .reserve(DONOR_IDENTITY_TABLE_CAPACITY);
            ledger
                .donor_identity_paths
                .reserve(DONOR_IDENTITY_TABLE_CAPACITY);
            ledger.prepared = true;
        });
    }

    pub(crate) fn reset() {
        // Do not replace the maps: dropping them would deallocate the warm-up buckets and the
        // next command would reallocate them inside the allocator measurement window.
        with_ledger(|ledger| {
            ledger.source_tokens.clear();
            ledger.source_path_tables.clear();
            ledger.generic_source_tokens.clear();
            ledger.donor_identity_strings.clear();
            ledger.donor_identity_paths.clear();
            ledger.source_totals = SourceTokenTotals::default();
            ledger.generic_source_tokens_owners = 0;
            ledger.generic_source_tokens_bytes = 0;
            ledger.generic_source_tokens_peak_bytes = 0;
            ledger.donor_identity_string_tables = 0;
            ledger.donor_identity_string_storage_bytes = 0;
            ledger.donor_identity_path_tables = 0;
            ledger.donor_identity_path_storage_bytes = 0;
            ledger.transient_construction_buffer_bytes = 0;
            ledger.transient_construction_peak_bytes = 0;
            ledger.requester_remap_count = 0;
            ledger.requester_remap_used_bytes = 0;
            ledger.requester_remap_capacity_bytes = 0;
            ledger.observation_incomplete = false;
            ledger.last_snapshot = None;
        });
    }

    fn register_source_path_table(
        ledger: &mut MemoryLedger,
        address: usize,
        store: StoreBytes,
    ) -> bool {
        if ledger.source_path_tables.contains_key(&address) {
            return true;
        }
        if ledger.source_path_tables.len() >= ledger.source_path_tables.capacity() {
            ledger.observation_incomplete = true;
            return false;
        }
        ledger.source_path_tables.insert(address, store);
        ledger.source_totals.path_store_bytes = ledger
            .source_totals
            .path_store_bytes
            .saturating_add(store.bytes);
        ledger.source_totals.path_store_capacity_bytes = ledger
            .source_totals
            .path_store_capacity_bytes
            .saturating_add(store.capacity_bytes);
        true
    }

    pub(crate) fn record_source_tokens(tokens: &SourceTokens) {
        let address = tokens as *const SourceTokens as usize;
        let metrics: SourceTokensMemoryLedgerMetrics = tokens.memory_ledger_metrics();
        with_ledger(|ledger| {
            if !ledger.prepared {
                return;
            }
            if let Some(previous) = ledger.source_tokens.get(&address).copied() {
                ledger.source_totals.shape_bytes = ledger
                    .source_totals
                    .shape_bytes
                    .saturating_sub(previous.shape_bytes)
                    .saturating_add(metrics.shape_bytes);
                ledger.source_totals.shape_capacity_bytes = ledger
                    .source_totals
                    .shape_capacity_bytes
                    .saturating_sub(previous.shape_capacity_bytes)
                    .saturating_add(metrics.shape_capacity_bytes);
                ledger.source_totals.span_bytes = ledger
                    .source_totals
                    .span_bytes
                    .saturating_sub(previous.span_bytes)
                    .saturating_add(metrics.span_bytes);
                ledger.source_totals.span_capacity_bytes = ledger
                    .source_totals
                    .span_capacity_bytes
                    .saturating_sub(previous.span_capacity_bytes)
                    .saturating_add(metrics.span_capacity_bytes);
                ledger.source_totals.numeric_store_bytes = ledger
                    .source_totals
                    .numeric_store_bytes
                    .saturating_sub(previous.numeric_store_bytes)
                    .saturating_add(metrics.numeric_store_bytes);
                ledger.source_totals.numeric_store_capacity_bytes = ledger
                    .source_totals
                    .numeric_store_capacity_bytes
                    .saturating_sub(previous.numeric_store_capacity_bytes)
                    .saturating_add(metrics.numeric_store_capacity_bytes);
                ledger.source_totals.sequence_store_bytes = ledger
                    .source_totals
                    .sequence_store_bytes
                    .saturating_sub(previous.sequence_store_bytes)
                    .saturating_add(metrics.sequence_store_bytes);
                ledger.source_totals.sequence_store_capacity_bytes = ledger
                    .source_totals
                    .sequence_store_capacity_bytes
                    .saturating_sub(previous.sequence_store_capacity_bytes)
                    .saturating_add(metrics.sequence_store_capacity_bytes);

                let mut path_table = previous.path_table;
                if path_table.is_none()
                    && let Some((path_address, bytes, capacity_bytes)) = metrics.path_table
                {
                    let store = StoreBytes {
                        bytes,
                        capacity_bytes,
                    };
                    if register_source_path_table(ledger, path_address, store) {
                        path_table = Some((path_address, store));
                    }
                }
                if let Some(owner) = ledger.source_tokens.get_mut(&address) {
                    *owner = SourceTokenOwner {
                        shape_bytes: metrics.shape_bytes,
                        shape_capacity_bytes: metrics.shape_capacity_bytes,
                        span_bytes: metrics.span_bytes,
                        span_capacity_bytes: metrics.span_capacity_bytes,
                        numeric_store_bytes: metrics.numeric_store_bytes,
                        numeric_store_capacity_bytes: metrics.numeric_store_capacity_bytes,
                        sequence_store_bytes: metrics.sequence_store_bytes,
                        sequence_store_capacity_bytes: metrics.sequence_store_capacity_bytes,
                        path_table,
                    };
                }
                return;
            }
            if ledger.source_tokens.len() >= ledger.source_tokens.capacity() {
                ledger.observation_incomplete = true;
                return;
            }
            let path_table = metrics.path_table.map(|(path_address, bytes, capacity_bytes)| {
                let store = StoreBytes {
                    bytes,
                    capacity_bytes,
                };
                (path_address, store)
            });
            if let Some((path_address, store)) = path_table {
                register_source_path_table(ledger, path_address, store);
            }
            let owner = SourceTokenOwner {
                shape_bytes: metrics.shape_bytes,
                shape_capacity_bytes: metrics.shape_capacity_bytes,
                span_bytes: metrics.span_bytes,
                span_capacity_bytes: metrics.span_capacity_bytes,
                numeric_store_bytes: metrics.numeric_store_bytes,
                numeric_store_capacity_bytes: metrics.numeric_store_capacity_bytes,
                sequence_store_bytes: metrics.sequence_store_bytes,
                sequence_store_capacity_bytes: metrics.sequence_store_capacity_bytes,
                path_table,
            };
            ledger.source_tokens.insert(address, owner);
            ledger.source_totals.owners = ledger.source_totals.owners.saturating_add(1);
            ledger.source_totals.shape_bytes = ledger
                .source_totals
                .shape_bytes
                .saturating_add(metrics.shape_bytes);
            ledger.source_totals.shape_capacity_bytes = ledger
                .source_totals
                .shape_capacity_bytes
                .saturating_add(metrics.shape_capacity_bytes);
            ledger.source_totals.span_bytes = ledger
                .source_totals
                .span_bytes
                .saturating_add(metrics.span_bytes);
            ledger.source_totals.span_capacity_bytes = ledger
                .source_totals
                .span_capacity_bytes
                .saturating_add(metrics.span_capacity_bytes);
            ledger.source_totals.numeric_store_bytes = ledger
                .source_totals
                .numeric_store_bytes
                .saturating_add(metrics.numeric_store_bytes);
            ledger.source_totals.numeric_store_capacity_bytes = ledger
                .source_totals
                .numeric_store_capacity_bytes
                .saturating_add(metrics.numeric_store_capacity_bytes);
            ledger.source_totals.sequence_store_bytes = ledger
                .source_totals
                .sequence_store_bytes
                .saturating_add(metrics.sequence_store_bytes);
            ledger.source_totals.sequence_store_capacity_bytes = ledger
                .source_totals
                .sequence_store_capacity_bytes
                .saturating_add(metrics.sequence_store_capacity_bytes);
        });
    }

    pub(crate) fn release_source_tokens(tokens: &SourceTokens) {
        let address = tokens as *const SourceTokens as usize;
        with_ledger(|ledger| {
            ledger.source_tokens.remove(&address);
            // Generic identity uses the same published SourceTokens allocation. Retire its live
            // key only when that allocation itself drops, not when one StableBodySyntax drops.
            ledger.generic_source_tokens.remove(&address);
        });
    }

    pub(crate) fn record_source_tokens_path_table(
        tokens: &SourceTokens,
        path_table: &PathSyntaxTable,
    ) {
        record_source_tokens(tokens);
        let owner_address = tokens as *const SourceTokens as usize;
        let path_address = path_table as *const PathSyntaxTable as usize;
        let store = StoreBytes {
            bytes: path_table.used_bytes(),
            capacity_bytes: path_table.storage_bytes(),
        };
        with_ledger(|ledger| {
            if !ledger.prepared {
                return;
            }
            if register_source_path_table(ledger, path_address, store)
                && let Some(owner) = ledger.source_tokens.get_mut(&owner_address)
                && owner.path_table.is_none()
            {
                owner.path_table = Some((path_address, store));
            }
        });
    }

    fn live_generic_source_token_bytes(ledger: &MemoryLedger) -> usize {
        ledger
            .generic_source_tokens
            .values()
            .filter(|owner| owner.live_references > 0)
            .map(|owner| owner.bytes)
            .sum()
    }

    fn record_generic_source_tokens_inner(tokens: &Arc<SourceTokens>, add_live_reference: bool) {
        record_source_tokens(tokens);
        let address = Arc::as_ptr(tokens) as usize;
        let bytes = tokens.memory_ledger_total_bytes();
        with_ledger(|ledger| {
            if !ledger.prepared {
                return;
            }
            if let Some(owner) = ledger.generic_source_tokens.get_mut(&address) {
                if add_live_reference {
                    owner.live_references = owner.live_references.saturating_add(1);
                }
            } else {
                if ledger.generic_source_tokens.len() >= ledger.generic_source_tokens.capacity() {
                    ledger.observation_incomplete = true;
                    return;
                }
                ledger.generic_source_tokens.insert(
                    address,
                    GenericSourceTokenOwner {
                        bytes,
                        live_references: usize::from(add_live_reference),
                    },
                );
                ledger.generic_source_tokens_owners = ledger
                    .generic_source_tokens_owners
                    .saturating_add(1);
                ledger.generic_source_tokens_bytes = ledger
                    .generic_source_tokens_bytes
                    .saturating_add(bytes);
            }
            ledger.generic_source_tokens_peak_bytes = ledger
                .generic_source_tokens_peak_bytes
                .max(live_generic_source_token_bytes(ledger));
        });
    }

    pub(crate) fn record_generic_source_tokens(tokens: &Arc<SourceTokens>) {
        record_generic_source_tokens_inner(tokens, true);
    }

    pub(crate) fn observe_generic_source_tokens(tokens: &Arc<SourceTokens>) {
        record_generic_source_tokens_inner(tokens, false);
    }

    pub(crate) fn release_generic_source_tokens(tokens: &Arc<SourceTokens>) {
        let address = Arc::as_ptr(tokens) as usize;
        with_ledger(|ledger| {
            if let Some(owner) = ledger.generic_source_tokens.get_mut(&address) {
                owner.live_references = owner.live_references.saturating_sub(1);
            }
        });
    }

    pub(crate) fn record_transient_construction_buffer_bytes(bytes: usize) {
        with_ledger(|ledger| {
            if !ledger.prepared {
                return;
            }
            ledger.transient_construction_buffer_bytes = ledger
                .transient_construction_buffer_bytes
                .saturating_add(bytes);
            ledger.transient_construction_peak_bytes = ledger
                .transient_construction_peak_bytes
                .max(bytes);
        });
    }

    pub(crate) fn record_requester_remap(remap: &PathIdRemap) {
        with_ledger(|ledger| {
            if !ledger.prepared {
                return;
            }
            ledger.requester_remap_count = ledger.requester_remap_count.saturating_add(1);
            ledger.requester_remap_used_bytes = ledger
                .requester_remap_used_bytes
                .saturating_add(remap.mapped_bytes());
            ledger.requester_remap_capacity_bytes = ledger
                .requester_remap_capacity_bytes
                .saturating_add(remap.mapped_capacity_bytes());
        });
    }

    pub(crate) fn record_donor_identity_tables(
        path_table: Option<&PathTable>,
        string_table: Option<&FrozenStringTable>,
    ) {
        with_ledger(|ledger| {
            if !ledger.prepared {
                return;
            }
            if let Some(path_table) = path_table {
                let address = path_table as *const PathTable as usize;
                if !ledger.donor_identity_paths.contains_key(&address) {
                    if ledger.donor_identity_paths.len() >= ledger.donor_identity_paths.capacity() {
                        ledger.observation_incomplete = true;
                    } else {
                        ledger.donor_identity_paths.insert(address, path_table.storage_bytes());
                        ledger.donor_identity_path_tables = ledger
                            .donor_identity_path_tables
                            .saturating_add(1);
                        ledger.donor_identity_path_storage_bytes = ledger
                            .donor_identity_path_storage_bytes
                            .saturating_add(path_table.storage_bytes());
                    }
                }
            }
            if let Some(string_table) = string_table {
                let address = string_table as *const FrozenStringTable as usize;
                if !ledger.donor_identity_strings.contains_key(&address) {
                    if ledger.donor_identity_strings.len() >= ledger.donor_identity_strings.capacity() {
                        ledger.observation_incomplete = true;
                    } else {
                        ledger
                            .donor_identity_strings
                            .insert(address, string_table.storage_bytes());
                        ledger.donor_identity_string_tables = ledger
                            .donor_identity_string_tables
                            .saturating_add(1);
                        ledger.donor_identity_string_storage_bytes = ledger
                            .donor_identity_string_storage_bytes
                            .saturating_add(string_table.storage_bytes());
                    }
                }
            }
        });
    }

    pub(crate) fn release_donor_identity_path(path_table: &PathTable) {
        let address = path_table as *const PathTable as usize;
        with_ledger(|ledger| {
            ledger.donor_identity_paths.remove(&address);
        });
    }

    pub(crate) fn release_donor_identity_string(string_table: &FrozenStringTable) {
        let address = string_table as *const FrozenStringTable as usize;
        with_ledger(|ledger| {
            ledger.donor_identity_strings.remove(&address);
        });
    }
    pub(crate) fn release_source_path_table(path_table: &PathSyntaxTable) {
        let address = path_table as *const PathSyntaxTable as usize;
        with_ledger(|ledger| {
            ledger.source_path_tables.remove(&address);
        });
    }

    pub(crate) fn snapshot() -> MemoryLedgerSnapshot {
        with_ledger(|ledger| {
            if let Some(snapshot) = ledger.last_snapshot {
                return snapshot;
            }
            let totals = ledger.source_totals;
            let snapshot = MemoryLedgerSnapshot {
                source_tokens_owners: totals.owners,
                source_tokens_shape_bytes: totals.shape_bytes,
                source_tokens_shape_capacity_bytes: totals.shape_capacity_bytes,
                source_tokens_span_bytes: totals.span_bytes,
                source_tokens_span_capacity_bytes: totals.span_capacity_bytes,
                source_tokens_numeric_store_bytes: totals.numeric_store_bytes,
                source_tokens_numeric_store_capacity_bytes: totals.numeric_store_capacity_bytes,
                source_tokens_path_store_bytes: totals.path_store_bytes,
                source_tokens_path_store_capacity_bytes: totals.path_store_capacity_bytes,
                source_tokens_sequence_store_bytes: totals.sequence_store_bytes,
                source_tokens_sequence_store_capacity_bytes: totals.sequence_store_capacity_bytes,
                transient_construction_buffer_bytes: ledger.transient_construction_buffer_bytes,
                transient_construction_peak_bytes: ledger.transient_construction_peak_bytes,
                // R4 removed the production FileTokens adapter. Keep this explicit field stable so
                // the probe can prove that the cutover did not reintroduce an adapter allocation.
                cutover_adapter_bytes: 0,
                donor_identity_string_tables: ledger.donor_identity_string_tables,
                donor_identity_string_storage_bytes: ledger.donor_identity_string_storage_bytes,
                donor_identity_path_tables: ledger.donor_identity_path_tables,
                donor_identity_path_storage_bytes: ledger.donor_identity_path_storage_bytes,
                generic_source_tokens_owners: ledger.generic_source_tokens_owners,
                generic_source_tokens_bytes: ledger.generic_source_tokens_bytes,
                generic_source_tokens_live_bytes: live_generic_source_token_bytes(ledger),
                generic_source_tokens_peak_bytes: ledger.generic_source_tokens_peak_bytes,
                requester_remap_count: ledger.requester_remap_count,
                requester_remap_used_bytes: ledger.requester_remap_used_bytes,
                requester_remap_capacity_bytes: ledger.requester_remap_capacity_bytes,
                observation_incomplete: ledger.observation_incomplete,
            };
            ledger.last_snapshot = Some(snapshot);
            snapshot
        })
    }
}

#[cfg(not(feature = "data_layout_memory_probe"))]
mod enabled {
    pub(crate) fn reset() {}
}

#[cfg(feature = "data_layout_memory_probe")]
pub(crate) use enabled::{
    observe_generic_source_tokens, prepare, record_donor_identity_tables,
    record_generic_source_tokens, record_requester_remap, record_source_tokens,
    record_source_tokens_path_table, record_transient_construction_buffer_bytes,
    release_donor_identity_path, release_donor_identity_string, release_generic_source_tokens,
    release_source_path_table, release_source_tokens, reset as reset_memory_ledger,
    snapshot as snapshot_memory_ledger,
};
#[cfg(not(feature = "data_layout_memory_probe"))]
pub(crate) use enabled::reset as reset_memory_ledger;
