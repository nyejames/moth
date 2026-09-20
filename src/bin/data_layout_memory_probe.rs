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
            let next = match current.checked_sub(bytes) {
                Some(next) => next,
                None => 0,
            };
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
            Self::add_live(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            Self::add_live(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        Self::remove_live(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
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
    println!(
        "retained.source_tokens_owners={}",
        retention.source_tokens_owners
    );
    println!(
        "retained.source_tokens_shape_bytes={}",
        retention.source_tokens_shape_bytes
    );
    println!(
        "retained.source_tokens_shape_capacity_bytes={}",
        retention.source_tokens_shape_capacity_bytes
    );
    println!(
        "retained.source_tokens_span_bytes={}",
        retention.source_tokens_span_bytes
    );
    println!(
        "retained.source_tokens_span_capacity_bytes={}",
        retention.source_tokens_span_capacity_bytes
    );
    println!(
        "retained.source_tokens_numeric_store_bytes={}",
        retention.source_tokens_numeric_store_bytes
    );
    println!(
        "retained.source_tokens_numeric_store_capacity_bytes={}",
        retention.source_tokens_numeric_store_capacity_bytes
    );
    println!(
        "retained.source_tokens_path_store_bytes={}",
        retention.source_tokens_path_store_bytes
    );
    println!(
        "retained.source_tokens_path_store_capacity_bytes={}",
        retention.source_tokens_path_store_capacity_bytes
    );
    println!(
        "retained.source_tokens_sequence_store_bytes={}",
        retention.source_tokens_sequence_store_bytes
    );
    println!(
        "retained.source_tokens_sequence_store_capacity_bytes={}",
        retention.source_tokens_sequence_store_capacity_bytes
    );
    println!(
        "retained.transient_construction_buffer_bytes={}",
        retention.transient_construction_buffer_bytes
    );
    println!(
        "retained.transient_construction_peak_bytes={}",
        retention.transient_construction_peak_bytes
    );
    println!(
        "retained.cutover_adapter_bytes={}",
        retention.cutover_adapter_bytes
    );
    println!(
        "retained.donor_identity_string_tables={}",
        retention.donor_identity_string_tables
    );
    println!(
        "retained.donor_identity_string_storage_bytes={}",
        retention.donor_identity_string_storage_bytes
    );
    println!(
        "retained.donor_identity_path_tables={}",
        retention.donor_identity_path_tables
    );
    println!(
        "retained.donor_identity_path_storage_bytes={}",
        retention.donor_identity_path_storage_bytes
    );
    println!(
        "retained.generic_source_tokens_owners={}",
        retention.generic_source_tokens_owners
    );
    println!(
        "retained.generic_source_tokens_bytes={}",
        retention.generic_source_tokens_bytes
    );
    println!(
        "retained.generic_source_tokens_live_bytes={}",
        retention.generic_source_tokens_live_bytes
    );
    println!(
        "retained.generic_source_tokens_peak_bytes={}",
        retention.generic_source_tokens_peak_bytes
    );
    println!(
        "retained.requester_remap_count={}",
        retention.requester_remap_count
    );
    println!(
        "retained.requester_remap_used_bytes={}",
        retention.requester_remap_used_bytes
    );
    println!(
        "retained.requester_remap_capacity_bytes={}",
        retention.requester_remap_capacity_bytes
    );
    println!(
        "retained.memory_ledger_incomplete={}",
        retention.memory_ledger_incomplete
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
    println!(
        "retained.source_snapshot_bytes={}",
        report.retention.source_snapshot_bytes
    );
    println!(
        "retained.extended_span_rows={}",
        report.retention.extended_span_rows
    );
    println!(
        "retained.source_identity_slots={}",
        report.retention.source_identity_slots
    );
    println!(
        "retained.diagnostic_records={}",
        report.retention.diagnostic_records
    );
    println!(
        "retained.diagnostic_label_slots={}",
        report.retention.diagnostic_label_slots
    );
    println!(
        "retained.identity_contexts={}",
        report.retention.retained_identity_contexts
    );
    println!(
        "retained.path_table_count={}",
        report.retention.path_table_count
    );
    println!(
        "retained.path_table_node_rows={}",
        report.retention.path_table_node_rows
    );
    println!(
        "retained.path_table_storage_bytes={}",
        report.retention.path_table_storage_bytes
    );
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

    match result {
        Ok(run) => {
            // The owner-preserving benchmark result keeps CompilerMessages and its frozen source
            // contexts alive while this sample is taken. `into_report` then drops that owner;
            // dropping the returned report separately gives the after-report-drop sample its
            // explicit lifetime rather than calling it "live".
            let live = LIVE_BYTES.load(Ordering::Relaxed);
            let live_report_bytes_delta = live.saturating_sub(baseline);
            print_report(
                &run.report,
                live_report_bytes_delta,
                peak_allocation_bytes_delta,
            );
            let report = run.into_report();
            drop(report);
            let after = LIVE_BYTES.load(Ordering::Relaxed);
            println!(
                "after_report_drop_bytes_delta={}",
                after.saturating_sub(baseline)
            );
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
            eprintln!("frontend benchmark failed: {error}");
            std::process::exit(1);
        }
    }
}
