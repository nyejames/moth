use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Process-local allocation accounting for this benchmark-only binary.
///
/// `peak_allocation_bytes_delta` is sampled from the high-water counter, while the probe samples
/// `live_report_bytes_delta` before dropping the returned render owner and
/// `after_report_drop_bytes_delta` after dropping both owner and report. These counters are
/// aggregate allocator proxies; they do not attribute bytes to compiler owners.
struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOC_SUCCESS_CALLS: AtomicUsize = AtomicUsize::new(0);
static ALLOC_ZEROED_SUCCESS_CALLS: AtomicUsize = AtomicUsize::new(0);
static REALLOC_SUCCESS_CALLS: AtomicUsize = AtomicUsize::new(0);
static DEALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
struct AllocatorCounts {
    alloc: usize,
    alloc_zeroed: usize,
    realloc: usize,
    dealloc: usize,
}

impl AllocatorCounts {
    fn snapshot() -> Self {
        Self {
            alloc: ALLOC_SUCCESS_CALLS.load(Ordering::Relaxed),
            alloc_zeroed: ALLOC_ZEROED_SUCCESS_CALLS.load(Ordering::Relaxed),
            realloc: REALLOC_SUCCESS_CALLS.load(Ordering::Relaxed),
            dealloc: DEALLOC_CALLS.load(Ordering::Relaxed),
        }
    }

    fn delta(self, baseline: Self) -> Self {
        Self {
            alloc: self.alloc.saturating_sub(baseline.alloc),
            alloc_zeroed: self.alloc_zeroed.saturating_sub(baseline.alloc_zeroed),
            realloc: self.realloc.saturating_sub(baseline.realloc),
            dealloc: self.dealloc.saturating_sub(baseline.dealloc),
        }
    }

    fn print(self) {
        println!("allocation.alloc_success_calls={}", self.alloc);
        println!(
            "allocation.alloc_zeroed_success_calls={}",
            self.alloc_zeroed
        );
        println!("allocation.realloc_success_calls={}", self.realloc);
        println!("allocation.dealloc_calls={}", self.dealloc);
        println!(
            "allocation.allocation_success_calls={}",
            self.alloc
                .saturating_add(self.alloc_zeroed)
                .saturating_add(self.realloc)
        );
    }
}

impl CountingAllocator {
    fn add_live(bytes: usize) {
        let mut current = LIVE_BYTES.load(Ordering::Relaxed);
        loop {
            let next = match current.checked_add(bytes) {
                Some(next) => next,
                None => usize::MAX,
            };
            match LIVE_BYTES.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    Self::record_peak(next);
                    return;
                }
                Err(observed) => current = observed,
            }
        }
    }

    fn remove_live(bytes: usize) {
        let mut current = LIVE_BYTES.load(Ordering::Relaxed);
        loop {
            // A mismatched deallocation must not wrap the accounting counter. This branch is only
            // a defensive accounting fallback; the allocator contract still belongs to System.
            let next = current.saturating_sub(bytes);
            match LIVE_BYTES.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    fn record_peak(live: usize) {
        let mut peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
        while live > peak {
            match PEAK_LIVE_BYTES.compare_exchange_weak(
                peak,
                live,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => peak = observed,
            }
        }
    }

    /// Start a measurement without pretending that allocations made before the probe disappeared.
    /// Keeping the live baseline allows deallocations to remain correctly paired with allocations
    /// made before this process entered the benchmark.
    fn reset_measurement() -> usize {
        let baseline = LIVE_BYTES.load(Ordering::Relaxed);
        PEAK_LIVE_BYTES.store(baseline, Ordering::Relaxed);
        baseline
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            ALLOC_SUCCESS_CALLS.fetch_add(1, Ordering::Relaxed);
            Self::add_live(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOC_ZEROED_SUCCESS_CALLS.fetch_add(1, Ordering::Relaxed);
            Self::add_live(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        DEALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        Self::remove_live(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            REALLOC_SUCCESS_CALLS.fetch_add(1, Ordering::Relaxed);
            if new_size >= layout.size() {
                Self::add_live(new_size - layout.size());
            } else {
                Self::remove_live(layout.size() - new_size);
            }
        }
        new_pointer
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn print_retention(retention: &moth::benchmarking::FrontendBenchmarkRetention) {
    macro_rules! print_fields {
        ($($field:ident),* $(,)?) => {
            $(
                println!("retained.{}={}", stringify!($field), retention.$field);
            )*
        };
    }
    print_fields!(
        source_snapshot_bytes,
        source_text_capacity_bytes,
        source_line_start_rows,
        source_line_start_bytes,
        source_line_start_capacity_bytes,
        extended_span_rows,
        extended_span_bytes,
        extended_span_duplicate_rows,
        source_identity_slots,
        source_identity_slot_capacity_bytes,
        source_provenance_field_bytes,
        compilation_root_slots,
        authored_physical_slots,
        canonical_os_path_bytes,
        loaded_source_records,
        loaded_source_record_capacity_bytes,
        diagnostic_records,
        diagnostic_record_capacity_bytes,
        diagnostic_label_slots,
        retained_identity_contexts,
        diagnostic_extra_rows,
        diagnostic_extra_capacity_bytes,
        diagnostic_label_rows,
        diagnostic_label_capacity_bytes,
        diagnostic_label_data_rows,
        diagnostic_label_data_capacity_bytes,
        diagnostic_string_list_rows,
        diagnostic_string_list_capacity_bytes,
        diagnostic_substitution_rows,
        diagnostic_substitution_capacity_bytes,
        diagnostic_foreign_site_rows,
        diagnostic_foreign_site_capacity_bytes,
        type_display_record_rows,
        type_display_record_capacity_bytes,
        type_display_auxiliary_rows,
        type_display_auxiliary_capacity_bytes,
        diagnostic_report_count,
        diagnostic_report_capacity_bytes,
        diagnostic_run_count,
        diagnostic_run_capacity_bytes,
        diagnostic_foreign_context_slots,
        diagnostic_foreign_context_capacity_bytes,
        frozen_context_shell_bytes,
        frozen_string_tables,
        frozen_string_storage_bytes,
        mutable_string_table_samples,
        mutable_string_table_slot_capacity_bytes,
        mutable_string_table_local_payload_bytes,
        mutable_string_table_reverse_capacity_entries,
        mutable_string_table_inherited_prefix_rows,
        mutable_string_table_inherited_prefix_slot_bytes,
        mutable_string_table_inherited_reverse_capacity_entries,
        mutable_string_table_peak_slot_capacity_bytes,
        mutable_path_builder_samples,
        mutable_path_table_storage_bytes,
        mutable_path_builder_lookup_layers,
        mutable_path_builder_lookup_capacity_entries,
        mutable_path_builder_local_lookup_capacity_entries,
        path_table_count,
        path_table_node_rows,
        path_table_storage_bytes,
        source_tokens_owners,
        source_tokens_shape_bytes,
        source_tokens_shape_capacity_bytes,
        source_tokens_span_bytes,
        source_tokens_span_capacity_bytes,
        source_tokens_numeric_store_bytes,
        source_tokens_numeric_store_capacity_bytes,
        source_tokens_path_store_bytes,
        source_tokens_path_store_capacity_bytes,
        source_tokens_sequence_store_bytes,
        source_tokens_sequence_store_capacity_bytes,
        transient_construction_buffer_bytes,
        transient_construction_peak_bytes,
        donor_identity_string_tables,
        donor_identity_string_storage_bytes,
        donor_identity_path_tables,
        donor_identity_path_storage_bytes,
        generic_source_tokens_owners,
        generic_source_tokens_bytes,
        generic_source_tokens_live_bytes,
        generic_source_tokens_peak_bytes,
        requester_remap_count,
        requester_remap_used_bytes,
        requester_remap_capacity_bytes,
        string_merge_remap_ns,
        path_merge_remap_ns,
        memory_ledger_incomplete,
    );
}

fn print_report(
    report: &moth::benchmarking::FrontendBenchmarkReport,
    live_report_bytes_delta: usize,
    peak_allocation_bytes_delta: usize,
) {
    println!("outcome={:?}", report.outcome);
    println!("errors={}", report.error_count);
    println!("warnings={}", report.warning_count);
    println!("total_ms={:.6}", report.total_ms);
    println!("live_report_bytes_delta={live_report_bytes_delta}");
    println!("peak_allocation_bytes_delta={peak_allocation_bytes_delta}");
    print_retention(&report.retention);
    for stage in &report.stages {
        println!("stage.{}={:.6}", stage.name, stage.duration_ms);
    }
    for counter in &report.counters {
        println!("counter.{}={}", counter.name, counter.value);
    }
}

fn print_error_metrics(
    elapsed_ms: f64,
    live_report_bytes_delta: usize,
    peak_allocation_bytes_delta: usize,
    after_report_drop_bytes_delta: usize,
    retention: &moth::benchmarking::FrontendBenchmarkRetention,
) {
    println!("outcome=Error");
    println!("errors=1");
    println!("warnings=0");
    println!("total_ms={elapsed_ms:.6}");
    println!("live_report_bytes_delta={live_report_bytes_delta}");
    println!("peak_allocation_bytes_delta={peak_allocation_bytes_delta}");
    println!("after_report_drop_bytes_delta={after_report_drop_bytes_delta}");
    print_retention(retention);
}
fn main() {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let Some(entry) = arguments.next() else {
        eprintln!("usage: data_layout_memory_probe <entry>");
        std::process::exit(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: data_layout_memory_probe <entry>");
        std::process::exit(2);
    }

    moth::benchmarking::prepare_frontend_memory_ledger();
    let allocation_baseline = AllocatorCounts::snapshot();
    let baseline = CountingAllocator::reset_measurement();
    let started = Instant::now();
    let result = moth::benchmarking::run_frontend_benchmark_with_report_owner(
        moth::benchmarking::FrontendBenchmarkOptions {
            entry_path: PathBuf::from(entry),
            build_profile: moth::benchmarking::FrontendBenchmarkBuildProfile::Dev,
            build_config_inputs: Vec::new(),
        },
    );
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let peak = PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    let peak_allocation_bytes_delta = peak.saturating_sub(baseline);
    let allocation_counts = AllocatorCounts::snapshot().delta(allocation_baseline);

    match result {
        Ok(run) => {
            // The owner-preserving benchmark result keeps its `CompilationResult<()>` and frozen
            // report/source contexts alive while this sample is taken. `into_report` then drops
            // that owner; dropping the returned report separately gives the after-report-drop
            // sample its explicit lifetime rather than calling it "live".
            let live = LIVE_BYTES.load(Ordering::Relaxed);
            let live_report_bytes_delta = live.saturating_sub(baseline);
            print_report(
                &run.report,
                live_report_bytes_delta,
                peak_allocation_bytes_delta,
            );
            let owner_release_started = Instant::now();
            let report = run.into_report();
            let owner_release_ns = owner_release_started.elapsed().as_nanos();
            let report_drop_started = Instant::now();
            drop(report);
            let report_drop_ns = report_drop_started.elapsed().as_nanos();
            let after = LIVE_BYTES.load(Ordering::Relaxed);
            allocation_counts.print();
            println!("owner_release_via_into_report_ns={owner_release_ns}");
            println!("report_drop_ns={report_drop_ns}");
            println!(
                "after_report_drop_bytes_delta={}",
                after.saturating_sub(baseline)
            );
            println!("rayon_threads={}", rayon::current_num_threads());
        }
        Err(error) => {
            let live = LIVE_BYTES.load(Ordering::Relaxed);
            let live_report_bytes_delta = live.saturating_sub(baseline);
            let after_report_drop_bytes_delta = live_report_bytes_delta;
            print_error_metrics(
                elapsed_ms,
                live_report_bytes_delta,
                peak_allocation_bytes_delta,
                after_report_drop_bytes_delta,
                &error.retention,
            );
            allocation_counts.print();
            println!("rayon_threads={}", rayon::current_num_threads());
            eprintln!("frontend benchmark failed: {error}");
            std::process::exit(1);
        }
    }
}
