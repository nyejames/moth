//! Central timing and benchmark-counter collector.
//!
//! WHAT: owns the crate-level collection scope that records named
//!      pipeline-stage durations and high-volume benchmark counters during
//!      compilation.
//! WHY:  lets concise timing runs happen behind the `timers` feature without
//!      pulling in the high-volume counters that flood output for large
//!      projects, and lets counter-only benchmark runs happen behind
//!      `benchmark_counters` without enabling verbose timer prose.
//!
//! Feature behavior:
//! - `#[cfg(not(feature = "timers"))]` — timing APIs compile as no-ops with
//!   zero runtime cost. Regular builds never touch `Instant`, atomics, or
//!   mutexes.
//! - `#[cfg(feature = "timers")]` — the collector is active and the `MOTH_TIMERS`
//!   environment variable controls timer output mode (off, summary, bench,
//!   verbose). `detailed_timers` implies `timers` and additionally enables
//!   verbose human timer prose and AST substage timings.
//! - `#[cfg(feature = "benchmark_counters")]` — high-volume frontend/AST
//!   counters are collected and the `MOTH_COUNTERS` environment variable
//!   controls counter output mode (off, summary, full). Counter storage
//!   requires the collector, so counters are only recorded when `timers` is
//!   also active. `detailed_timers` no longer enables counters by itself.
//!
//! Stage boundaries: this module owns timing and counter infrastructure only.
//! It must not import or depend on frontend, analysis, IR, or backend modules.
//! Compiler stages call into it through macros and the collector API.

// ---------------------------------------------------------------------------
//  Dead-code allowance for remaining unused timing APIs
// ---------------------------------------------------------------------------
// When `timers` is active but no caller records a particular observation,
// some collector-backed APIs can look unused to the compiler (for example
// `record_labeled_pipeline_timing`, which has no labeled call sites yet).
// This targeted allowance suppresses those expected dead-code warnings so
// `cargo check --features timers` stays quiet. Most public APIs now have
// real call sites from Phase 2 onward.
#![cfg_attr(feature = "timers", allow(dead_code))]
// Counter summary helpers are only called from the command timing summary
// (gated by `timers`); suppress dead-code warnings for `benchmark_counters`-only
// builds where no command summary runs.
#![cfg_attr(
    all(feature = "benchmark_counters", not(feature = "timers")),
    allow(dead_code)
)]

use std::time::Duration;

// ---------------------------------------------------------------------------
//  Shared observation types
// ---------------------------------------------------------------------------

/// One named metric value captured during a benchmark collection scope.
///
/// Used for both timings (value in milliseconds) and counters (raw count). The
/// combined snapshot keeps both in one type so the single collection scope can
/// serve in-process benchmark APIs that read timings and counters together.
#[cfg_attr(not(feature = "timers"), allow(dead_code))]
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkObservationMetric {
    pub(crate) name: String,
    pub(crate) value: f64,
    /// Optional attribution label for summary max display (e.g. slowest module).
    /// Never appears in stable `MOTH_BENCH timing` lines.
    pub(crate) label: Option<String>,
}

/// Snapshot of all observations captured in one collection scope.
///
/// `timings` is populated when the `timers` feature is active.
/// `counters` is only populated when both `timers` and `benchmark_counters`
/// are active, because counter storage reuses the same collector and is gated
/// behind `benchmark_counters`. `detailed_timers` alone no longer populates
/// counters.
#[cfg_attr(not(feature = "timers"), allow(dead_code))]
#[derive(Debug, Clone, Default)]
pub(crate) struct BenchmarkObservationSnapshot {
    pub(crate) timings: Vec<BenchmarkObservationMetric>,
    pub(crate) counters: Vec<BenchmarkObservationMetric>,
}

// ---------------------------------------------------------------------------
//  Active timing collection (feature = "timers")
// ---------------------------------------------------------------------------

/// Output mode controlling how timing information reaches the user.
///
/// Parsed from the `MOTH_TIMERS` environment variable. When unset,
/// `detailed_timers` defaults to `Verbose` (preserving existing behavior),
/// while `timers` alone defaults to `Summary`.
#[cfg(feature = "timers")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerOutputMode {
    /// No timing output at all.
    Silent,
    /// Concise human-readable summary printed after compilation.
    Summary,
    /// Stable machine-readable `MOTH_BENCH timing` lines for benchmark parsing.
    Bench,
    /// Both human prose and stable benchmark lines.
    Verbose,
}

#[cfg(feature = "timers")]
impl TimerOutputMode {
    /// Parse the output mode from the `MOTH_TIMERS` environment variable.
    ///
    /// When `MOTH_TIMERS` is unset, `detailed_timers` defaults to `Verbose`
    /// (backward compatible) and `timers` alone defaults to `Summary`.
    pub(crate) fn from_env() -> Self {
        match std::env::var("MOTH_TIMERS").as_deref() {
            Ok("silent") | Ok("none") | Ok("off") => Self::Silent,
            Ok("summary") => Self::Summary,
            Ok("bench") => Self::Bench,
            Ok("verbose") | Ok("full") => Self::Verbose,
            _ => {
                // Preserve existing detailed_timers behavior: verbose by default.
                // Timers-only builds default to a concise summary.
                #[cfg(feature = "detailed_timers")]
                {
                    Self::Verbose
                }
                #[cfg(not(feature = "detailed_timers"))]
                {
                    Self::Summary
                }
            }
        }
    }

    /// Whether stable `MOTH_BENCH timing` lines should be printed.
    pub(crate) fn emits_bench_lines(self) -> bool {
        matches!(self, Self::Bench | Self::Verbose)
    }

    /// Whether a human-readable summary should be printed.
    pub(crate) fn emits_summary(self) -> bool {
        matches!(self, Self::Summary | Self::Verbose)
    }

    /// Whether human timer prose should be printed inline during compilation.
    pub(crate) fn emits_human_prose(self) -> bool {
        matches!(self, Self::Verbose)
    }
}

// ---------------------------------------------------------------------------
//  Counter output mode (feature = "benchmark_counters")
// ---------------------------------------------------------------------------

/// Output mode controlling how high-volume benchmark counters reach the user.
///
/// Parsed from the `MOTH_COUNTERS` environment variable. Counters are always
/// collected into the central snapshot when `benchmark_counters` and `timers`
/// are both active; this mode only controls what reaches stdout.
///
/// - `Off` (default): collect counters but print nothing. Lets in-process
///   benchmark APIs read counters programmatically without flooding CLI output.
/// - `Summary`: emit stable `MOTH_BENCH counter` lines and print a concise
///   grouped counter summary after compilation.
/// - `Full`: emit stable `MOTH_BENCH counter` lines and print the legacy
///   per-counter human dump (the old `detailed_timers` behavior).
#[cfg(feature = "benchmark_counters")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CounterOutputMode {
    /// Collect counters but print no counter stdout.
    Off,
    /// Stable counter lines plus a concise grouped summary.
    Summary,
    /// Stable counter lines plus the legacy per-counter human dump.
    Full,
}

#[cfg(feature = "benchmark_counters")]
impl CounterOutputMode {
    /// Parse the output mode from the `MOTH_COUNTERS` environment variable.
    ///
    /// Unset or unrecognized values default to `Off` so regular benchmark
    /// builds do not flood stdout with counter prose.
    pub(crate) fn from_env() -> Self {
        match std::env::var("MOTH_COUNTERS").as_deref() {
            Ok("summary") => Self::Summary,
            Ok("full") => Self::Full,
            _ => Self::Off,
        }
    }

    /// Whether stable `MOTH_BENCH counter` lines should be printed.
    pub(crate) fn emits_bench_counter_lines(self) -> bool {
        matches!(self, Self::Summary | Self::Full)
    }

    /// Whether the concise grouped counter summary should be printed after
    /// compilation.
    pub(crate) fn emits_counter_summary(self) -> bool {
        matches!(self, Self::Summary)
    }

    /// Whether the legacy per-counter human dump should be printed while
    /// counters are logged.
    pub(crate) fn emits_human_counter_prose(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Aggregated view of repeated timing observations for summary output.
///
/// WHAT: combines multiple observations with the same stable metric name into
/// one total/count/max line.
/// WHY: project-level timing summaries must stay short even when later phases
/// record per-module or per-file metrics.
#[cfg(feature = "timers")]
#[derive(Debug, Clone, Default)]
pub(crate) struct TimingMetricSummary {
    pub(crate) total_ms: f64,
    pub(crate) count: u64,
    pub(crate) max_ms: f64,
    /// Attribution label of the slowest sample, shown in the concise summary.
    pub(crate) max_label: Option<String>,
}

#[cfg(feature = "timers")]
impl TimingMetricSummary {
    fn record(&mut self, value_ms: f64, label: Option<&str>) {
        self.total_ms += value_ms;
        let is_first_sample = self.count == 0;
        self.count += 1;
        if is_first_sample || value_ms > self.max_ms {
            self.max_ms = value_ms;
            self.max_label = label.map(|text| text.to_owned());
        }
    }
}

/// Thread-safe in-memory collector for benchmark timing and counter observations.
///
/// WHAT: captures stable benchmark metric values during an active collection scope
/// so that in-process benchmark APIs can read timings and counters directly
/// instead of parsing stdout.
/// WHY: subprocess-free frontend benchmarks need programmatic access to the same
/// metrics that CLI benchmarks extract from stable `MOTH_BENCH` lines.
#[cfg(feature = "timers")]
mod collector {
    use super::{BenchmarkObservationMetric, BenchmarkObservationSnapshot};
    use std::sync::Mutex;

    struct ActiveCollection {
        timings: Vec<BenchmarkObservationMetric>,
        counters: Vec<BenchmarkObservationMetric>,
        suppress_output: bool,
    }

    static ACTIVE_COLLECTOR: Mutex<Option<ActiveCollection>> = Mutex::new(None);

    /// Start a new collection scope, discarding any previous in-flight data.
    ///
    /// When `suppress_output` is true, all stdout output is suppressed while
    /// observations are still recorded. This is used by in-process benchmarks
    /// that read observations programmatically instead of parsing stdout.
    pub(crate) fn start_collection(suppress_output: bool) {
        if let Ok(mut guard) = ACTIVE_COLLECTOR.lock() {
            *guard = Some(ActiveCollection {
                timings: Vec::new(),
                counters: Vec::new(),
                suppress_output,
            });
        }
    }

    /// Record one timing observation if a collection scope is active.
    pub(crate) fn record_timing(name: &str, millis: f64) {
        if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
            && let Some(collection) = guard.as_mut()
        {
            collection.timings.push(BenchmarkObservationMetric {
                name: name.to_string(),
                value: millis,
                label: None,
            });
        }
    }

    /// Record one timing observation with an attribution label.
    ///
    /// The label is stored for summary max display only; it never appears in
    /// stable `MOTH_BENCH timing` lines so benchmark parsing is unaffected.
    pub(crate) fn record_labeled_timing(name: &str, millis: f64, label: &str) {
        if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
            && let Some(collection) = guard.as_mut()
        {
            collection.timings.push(BenchmarkObservationMetric {
                name: name.to_string(),
                value: millis,
                label: Some(label.to_owned()),
            });
        }
    }

    /// Record one counter observation if a collection scope is active.
    ///
    /// The public `record_counter` wrapper is gated behind `benchmark_counters`,
    /// so this is only reached when both `timers` (the collector) and
    /// `benchmark_counters` are active. `detailed_timers` alone no longer
    /// routes counters here.
    pub(crate) fn record_counter(name: &str, value: f64) {
        if let Ok(mut guard) = ACTIVE_COLLECTOR.lock()
            && let Some(collection) = guard.as_mut()
        {
            collection.counters.push(BenchmarkObservationMetric {
                name: name.to_string(),
                value,
                label: None,
            });
        }
    }

    /// Whether stdout output is currently allowed.
    ///
    /// Returns false when an in-process collection scope has suppressed output.
    /// Returns true when no scope is active (normal CLI compilation).
    pub(crate) fn output_enabled() -> bool {
        match ACTIVE_COLLECTOR.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(collection) => !collection.suppress_output,
                None => true,
            },
            Err(_) => true,
        }
    }

    /// Stop the current collection scope and return all captured observations.
    ///
    /// Returns an empty snapshot if no scope was active or if the lock was poisoned.
    pub(crate) fn stop_and_collect() -> BenchmarkObservationSnapshot {
        if let Ok(mut guard) = ACTIVE_COLLECTOR.lock() {
            guard
                .take()
                .map_or_else(BenchmarkObservationSnapshot::default, |collection| {
                    BenchmarkObservationSnapshot {
                        timings: collection.timings,
                        counters: collection.counters,
                    }
                })
        } else {
            BenchmarkObservationSnapshot::default()
        }
    }
}

// ---------------------------------------------------------------------------
//  Public timing API (feature = "timers")
// ---------------------------------------------------------------------------

/// Start a benchmark collection scope.
///
/// When `suppress_output` is true, stdout is suppressed while observations
/// are still recorded in the collector. This is used by in-process frontend
/// benchmarks that read observations programmatically.
#[cfg(feature = "timers")]
pub(crate) fn start_benchmark_collection(suppress_output: bool) {
    collector::start_collection(suppress_output);
}

/// Stop the current collection scope and return all captured observations.
///
/// Returns an empty snapshot when no scope was active.
#[cfg(feature = "timers")]
pub(crate) fn stop_and_collect_benchmark_observations() -> BenchmarkObservationSnapshot {
    collector::stop_and_collect()
}

/// Record one timing observation in the active collection scope.
///
/// Called by `compiler_dev_logging::log_benchmark_timing` and by the
/// `pipeline_timer!` / `labeled_pipeline_timer!` macros.
#[cfg(feature = "timers")]
pub(crate) fn record_timing(name: &str, millis: f64) {
    collector::record_timing(name, millis);
}

/// Record one counter observation in the active collection scope.
///
/// Called by `compiler_dev_logging::log_benchmark_counter` and by the
/// Stage 0 discovery paths. Counter storage reuses the `timers` collector, so
/// this is only active when both `timers` and `benchmark_counters` are on.
/// `detailed_timers` alone no longer routes counters here.
#[cfg(all(feature = "timers", feature = "benchmark_counters"))]
pub(crate) fn record_counter(name: &str, value: f64) {
    collector::record_counter(name, value);
}

/// Whether stdout output is currently allowed (not suppressed by an
/// in-process collection scope).
#[cfg(feature = "timers")]
pub(crate) fn output_enabled() -> bool {
    collector::output_enabled()
}

/// The current timer output mode parsed from `MOTH_TIMERS`.
#[cfg(feature = "timers")]
pub(crate) fn current_output_mode() -> TimerOutputMode {
    TimerOutputMode::from_env()
}

/// The current counter output mode parsed from `MOTH_COUNTERS`.
///
/// Counters are collected regardless of this mode (when `timers` and
/// `benchmark_counters` are both active); this only governs stdout.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn current_counter_output_mode() -> CounterOutputMode {
    CounterOutputMode::from_env()
}

/// Emit one stable `MOTH_BENCH counter` line to stdout if the counter output
/// mode permits and output is not suppressed.
///
/// WHAT: prints a plain `MOTH_BENCH counter <metric>=<value>` line that the
///      benchmark observation parser can grep without depending on human prose.
/// WHY:  like the timing line, separating the stable counter metric from
///       human prose lets counter logging change its display without breaking
///       benchmark attribution. The line is only emitted for `MOTH_COUNTERS`
///       modes that request stdout (`summary` or `full`).
#[cfg(feature = "benchmark_counters")]
pub(crate) fn emit_bench_counter_line(name: &str, value: f64) {
    if name.trim().is_empty() {
        return;
    }

    let mode = CounterOutputMode::from_env();

    if output_enabled() && mode.emits_bench_counter_lines() {
        saying::say!("MOTH_BENCH counter ", name, "=", #value);
    }
}

#[cfg(feature = "benchmark_counters")]
struct CounterSummaryGroup {
    label: &'static str,
    metrics: &'static [(&'static str, &'static str)],
}

#[cfg(feature = "benchmark_counters")]
const COUNTER_SUMMARY_GROUPS: &[CounterSummaryGroup] = &[
    CounterSummaryGroup {
        label: "inputs",
        metrics: &[
            ("module_count", "modules"),
            ("source_file_count", "files"),
            ("source_byte_count", "bytes"),
            ("prepared_file_count", "prepared"),
            ("token_count", "tokens"),
            ("header_count", "headers"),
            ("import_count", "imports"),
            ("top_level_declaration_count", "decls"),
        ],
    },
    CounterSummaryGroup {
        label: "stage0",
        metrics: &[
            ("source_tree_index.discovery_runs", "source scans"),
            ("source_tree_index.dirs_visited", "dirs"),
            ("source_tree_index.dirs_skipped", "skipped dirs"),
            ("source_tree_index.files_seen", "files seen"),
            ("source_tree_index.module_roots_found", "roots"),
            (
                "stage0.reachable_discovery.reachable_files",
                "reachable files",
            ),
            ("stage0.reachable_discovery.import_edges", "import edges"),
            ("stage0_source_cache_hit_count", "source hits"),
            ("stage0_source_cache_miss_count", "source misses"),
            ("stage0_source_bytes_loaded", "bytes loaded"),
        ],
    },
    CounterSummaryGroup {
        label: "scheduling",
        metrics: &[
            ("module_compilation_serial_count", "serial modules"),
            ("module_compilation_parallel_task_count", "parallel tasks"),
            ("file_preparation_serial_module_count", "serial file prep"),
            (
                "file_preparation_parallel_module_count",
                "parallel file prep",
            ),
            (
                "file_preparation_strategy_parallel_per_file_count",
                "per-file strategy",
            ),
            (
                "file_preparation_strategy_chunked_count",
                "chunked strategy",
            ),
        ],
    },
    CounterSummaryGroup {
        label: "frontend",
        metrics: &[
            ("dependency_header_count", "dep headers"),
            ("dependency_edge_count", "dep edges"),
            ("dependency_visit_count", "dep visits"),
            ("ast_header_count", "AST headers"),
            ("ast_function_count", "functions"),
            ("ast_struct_count", "structs"),
            ("ast_choice_count", "choices"),
            ("ast_constant_count", "constants"),
            ("ast_receiver_method_count", "receiver methods"),
            ("ast_generic_instance_count", "generic instances"),
            ("hir_block_count", "HIR blocks"),
            ("hir_statement_count", "HIR statements"),
            ("hir_function_count", "HIR functions"),
            ("borrow_function_count", "borrow functions"),
            ("borrow_block_count", "borrow blocks"),
            ("borrow_conflict_check_count", "borrow checks"),
            ("borrow_state_join_count", "borrow joins"),
            ("borrow_place_access_count", "borrow places"),
        ],
    },
    CounterSummaryGroup {
        label: "scope/type",
        metrics: &[
            ("actual_scope_frames", "scope frames"),
            ("scope_arena_capacity", "scope capacity"),
            (
                "type_environment_substitute_type_id_calls",
                "type substitutions",
            ),
            (
                "type_environment_substitution_cache_lookups",
                "substitution lookups",
            ),
            ("type_compatibility_cache_lookups", "compat lookups"),
            ("type_compatibility_cache_misses", "compat misses"),
        ],
    },
    CounterSummaryGroup {
        label: "string/remap",
        metrics: &[
            ("string_table_full_clones", "full clones"),
            ("string_table_merge_source_entries_scanned", "merge scanned"),
            ("string_table_delta_merge_calls", "delta merges"),
            ("string_table_delta_entries_scanned", "delta scanned"),
            (
                "string_table_delta_non_identity_remaps",
                "non-identity remaps",
            ),
            ("module_remap_string_ids_calls", "module remaps"),
            ("file_prepare_output_remap_calls", "file output remaps"),
            ("file_prepare_error_remap_calls", "file error remaps"),
        ],
    },
    CounterSummaryGroup {
        label: "templates/tir",
        metrics: &[
            ("template_count", "templates"),
            ("const_template_count", "const"),
            ("runtime_template_count", "runtime"),
            ("ast_template_atoms_parsed", "atoms"),
            (
                "ast_templates_folded_during_finalization",
                "finalized folds",
            ),
            ("ast_template_tir_sync_attempts", "TIR sync attempts"),
            ("ast_template_tir_sync_successes", "TIR sync success"),
            ("ast_tir_templates_created", "TIR templates"),
            ("ast_tir_nodes_created", "TIR nodes"),
            ("ast_tir_text_bytes_recorded", "TIR text bytes"),
            ("ast_tir_fold_nodes_visited", "TIR fold nodes"),
        ],
    },
    CounterSummaryGroup {
        label: "external packages",
        metrics: &[
            ("external_package_registry_clone_count", "registry clones"),
            ("external_package_definition_clone_count", "package clones"),
            (
                "external_function_definition_clone_count",
                "function clones",
            ),
            ("external_symbol_path_clone_count", "symbol clones"),
            ("external_abi_parameter_clone_count", "ABI clones"),
        ],
    },
];

/// Render a concise grouped counter summary from a collected snapshot.
///
/// Aggregates counter observations by metric name (summing repeated samples,
/// e.g. per-module discovery counters) and returns a small fixed set of
/// stage-oriented lines. Stable `MOTH_BENCH counter` output remains the full
/// machine-readable path; the human summary is deliberately compact.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn render_counter_summary(snapshot: &BenchmarkObservationSnapshot) -> Vec<String> {
    if snapshot.counters.is_empty() {
        return Vec::new();
    }

    let mut aggregates = std::collections::BTreeMap::<&str, f64>::new();
    for metric in &snapshot.counters {
        *aggregates.entry(metric.name.as_str()).or_default() += metric.value;
    }

    let mut lines = Vec::with_capacity(COUNTER_SUMMARY_GROUPS.len() + 2);
    lines.push("Counter summary:".to_owned());

    for group in COUNTER_SUMMARY_GROUPS {
        if let Some(line) = render_counter_summary_group(&aggregates, group) {
            lines.push(line);
        }
    }

    let other_nonzero_count = aggregates
        .iter()
        .filter(|(name, value)| **value != 0.0 && !counter_summary_includes_metric(name))
        .count();

    if other_nonzero_count > 0 {
        lines.push(format!(
            "  other nonzero counters: {other_nonzero_count} (see MOTH_BENCH lines)"
        ));
    }

    if lines.len() == 1 {
        lines.push("  no nonzero counters".to_owned());
    }

    lines
}

#[cfg(feature = "benchmark_counters")]
fn render_counter_summary_group(
    aggregates: &std::collections::BTreeMap<&str, f64>,
    group: &CounterSummaryGroup,
) -> Option<String> {
    let mut parts = Vec::new();

    for (metric_name, label) in group.metrics {
        let value = aggregates.get(metric_name).copied().unwrap_or(0.0);
        if value == 0.0 {
            continue;
        }
        parts.push(format!("{label} {}", format_counter_summary_value(value)));
    }

    if parts.is_empty() {
        None
    } else {
        Some(format!("  {}: {}", group.label, parts.join(", ")))
    }
}

#[cfg(feature = "benchmark_counters")]
fn counter_summary_includes_metric(name: &str) -> bool {
    COUNTER_SUMMARY_GROUPS.iter().any(|group| {
        group
            .metrics
            .iter()
            .any(|(metric_name, _label)| *metric_name == name)
    })
}

#[cfg(feature = "benchmark_counters")]
fn format_counter_summary_value(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

/// Emit one stable `MOTH_BENCH timing` line to stdout if the output mode
/// permits and output is not suppressed.
///
/// WHAT: prints a plain `MOTH_BENCH timing <metric>=<millis>ms` line that the
/// benchmark observation parser can grep without depending on human prose.
/// WHY: separating the stable metric line from colored human output lets
/// compiler logging change its prose without silently breaking attribution.
#[cfg(feature = "timers")]
pub(crate) fn emit_bench_timing_line(name: &str, duration: Duration) {
    if name.trim().is_empty() {
        return;
    }

    let mode = TimerOutputMode::from_env();

    if output_enabled() && mode.emits_bench_lines() {
        let millis = duration.as_secs_f64() * 1000.0;
        saying::say!("MOTH_BENCH timing ", name, "=", #millis, "ms");
    }
}

/// Record a pipeline-stage timing and emit the stable bench line when
/// appropriate.
///
/// Used by the `pipeline_timer!` macro. The timing is always recorded in the
/// collector (when a scope is active); the stdout line depends on the output
/// mode and suppression flag.
#[cfg(feature = "timers")]
pub(crate) fn record_pipeline_timing(metric: &str, duration: Duration) {
    let millis = duration.as_secs_f64() * 1000.0;
    record_timing(metric, millis);
    emit_bench_timing_line(metric, duration);
}

/// Opaque start token for manually timed pipeline stages.
///
/// WHAT: stores an `Instant` only when the `timers` feature is active.
/// WHY: command/build orchestration sometimes needs to record a duration after
///      branching over error paths, where expression-wrapping macros would make
///      the control flow harder to read. The no-feature token is zero-sized, so
///      regular builds do not pay for timer reads.
#[cfg(feature = "timers")]
pub(crate) type PipelineTimingStart = std::time::Instant;

#[cfg(not(feature = "timers"))]
#[derive(Clone, Copy)]
pub(crate) struct PipelineTimingStart;

/// Start a manually recorded pipeline stage.
#[cfg(feature = "timers")]
pub(crate) fn start_pipeline_timing() -> PipelineTimingStart {
    std::time::Instant::now()
}

/// No-op stage start token when `timers` is off.
#[cfg(not(feature = "timers"))]
pub(crate) fn start_pipeline_timing() -> PipelineTimingStart {
    PipelineTimingStart
}

/// Record a manually timed pipeline stage from a previously captured start token.
#[cfg(feature = "timers")]
pub(crate) fn record_started_pipeline_timing(metric: &str, start: PipelineTimingStart) {
    record_pipeline_timing(metric, start.elapsed());
}

/// Record a pipeline-stage timing with an optional attribution label.
///
/// WHAT: like `record_pipeline_timing` but stores an optional label alongside
///      the observation so the concise summary can show the slowest contributor.
/// WHY:  project-level frontend timings repeat per module; the label lets the
///       summary attribute the max sample without flooding output with per-module
///       lines.  The label never appears in stable `MOTH_BENCH timing` lines.
#[cfg(feature = "timers")]
pub(crate) fn record_pipeline_timing_with_label(
    metric: &str,
    duration: Duration,
    label: Option<&str>,
) {
    let millis = duration.as_secs_f64() * 1000.0;
    match label {
        Some(text) => collector::record_labeled_timing(metric, millis, text),
        None => collector::record_timing(metric, millis),
    }
    emit_bench_timing_line(metric, duration);
}

/// Record a manually timed pipeline stage with an optional attribution label.
#[cfg(feature = "timers")]
pub(crate) fn record_started_pipeline_timing_with_label(
    metric: &str,
    start: PipelineTimingStart,
    label: Option<&str>,
) {
    record_pipeline_timing_with_label(metric, start.elapsed(), label);
}

/// RAII guard that records a pipeline-stage timing when dropped.
///
/// WHAT: captures a start instant on construction and records the elapsed
///      duration under the given metric name when the guard goes out of scope.
/// WHY:  backend orchestration has many early-return error paths; a Drop guard
///      ensures every stage is timed without scattering explicit record calls
///      before every `return Err`.
#[cfg(feature = "timers")]
pub(crate) struct PipelineTimingGuard {
    metric: &'static str,
    start: PipelineTimingStart,
}

#[cfg(feature = "timers")]
impl PipelineTimingGuard {
    /// Start timing a stage that will be recorded when the guard drops.
    pub(crate) fn new(metric: &'static str) -> Self {
        Self {
            metric,
            start: start_pipeline_timing(),
        }
    }
}

#[cfg(feature = "timers")]
impl Drop for PipelineTimingGuard {
    fn drop(&mut self) {
        record_started_pipeline_timing(self.metric, self.start);
    }
}

/// No-op manual stage recorder when `timers` is off.
#[cfg(not(feature = "timers"))]
pub(crate) fn record_started_pipeline_timing(_metric: &str, _start: PipelineTimingStart) {}

/// No-op labeled pipeline timing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn record_pipeline_timing_with_label(
    _metric: &str,
    _duration: Duration,
    _label: Option<&str>,
) {
}

/// No-op labeled manual stage recorder when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn record_started_pipeline_timing_with_label(
    _metric: &str,
    _start: PipelineTimingStart,
    _label: Option<&str>,
) {
}

/// Zero-sized no-op timing guard when `timers` is off.
#[cfg(not(feature = "timers"))]
#[derive(Clone, Copy)]
pub(crate) struct PipelineTimingGuard;

#[cfg(not(feature = "timers"))]
impl PipelineTimingGuard {
    pub(crate) fn new(_metric: &'static str) -> Self {
        Self
    }
}

/// Record a pipeline-stage timing with a human-readable label and emit the
/// stable bench line when appropriate.
///
/// Used by the `labeled_pipeline_timer!` macro. The human label is printed
/// inline only in verbose mode; the stable bench line depends on the output
/// mode and suppression flag.
#[cfg(feature = "timers")]
pub(crate) fn record_labeled_pipeline_timing(metric: &str, duration: Duration, label: &str) {
    let mode = TimerOutputMode::from_env();

    if output_enabled() && mode.emits_human_prose() {
        saying::say!(label, Green #duration);
    }

    record_pipeline_timing(metric, duration);
}

/// Render a concise human-readable summary of all captured timings.
///
/// Returns one aggregate line per timing metric, sorted by name, suitable for
/// printing after compilation when the output mode is `Summary` or `Verbose`.
#[cfg(feature = "timers")]
pub(crate) fn render_timing_summary(snapshot: &BenchmarkObservationSnapshot) -> Vec<String> {
    if snapshot.timings.is_empty() {
        return Vec::new();
    }

    let mut aggregates = std::collections::BTreeMap::<&str, TimingMetricSummary>::new();
    for metric in &snapshot.timings {
        aggregates
            .entry(metric.name.as_str())
            .or_default()
            .record(metric.value, metric.label.as_deref());
    }

    let mut lines = Vec::with_capacity(aggregates.len() + 1);
    lines.push("Timing summary:".to_owned());

    for (name, summary) in aggregates {
        if summary.count == 1 {
            lines.push(format!("  {name}: {:.2}ms", summary.total_ms));
        } else if let Some(label) = &summary.max_label {
            lines.push(format!(
                "  {name}: {:.2}ms across {} samples; max {:.2}ms [{label}]",
                summary.total_ms, summary.count, summary.max_ms
            ));
        } else {
            lines.push(format!(
                "  {name}: {:.2}ms across {} samples; max {:.2}ms",
                summary.total_ms, summary.count, summary.max_ms
            ));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
//  Command-scope timing API
// ---------------------------------------------------------------------------

/// Start a command-level timing collection scope.
///
/// WHAT: begins collecting timing observations for a single CLI command run so
///      that `print_command_timing_summary` can render a concise summary.
/// WHY:  the summary printed after `check` and `build` reads from this scope.
///       Call this at the top of a command handler, before any stage work begins.
#[cfg(feature = "timers")]
pub(crate) fn start_command_timing() {
    start_benchmark_collection(false);
}

/// Stop the command timing scope and print the concise timing summary when the
/// output mode requests one.
///
/// WHAT: collects all recorded timings and prints the human-readable summary
///      after normal diagnostics/success output so timer prose never obscures
///      compiler messages.
/// WHY:  callers must print diagnostics first, then call this.  In `Bench` mode
///      the summary is skipped (stable `MOTH_BENCH timing` lines were already
///      emitted inline).  In `Silent` mode nothing is printed.  The collection
///      scope is always stopped to clean up even when no summary is shown.
#[cfg(feature = "timers")]
pub(crate) fn print_command_timing_summary() {
    let mode = TimerOutputMode::from_env();

    let snapshot = stop_and_collect_benchmark_observations();

    if mode.emits_summary() {
        for line in render_timing_summary(&snapshot) {
            saying::say!(line);
        }
    }

    // Counter summary is owned by `benchmark_counters` and reuses the snapshot
    // just drained by the timing summary. It only prints when `MOTH_COUNTERS`
    // requests the concise summary view; the legacy full dump is printed inline
    // while counters are logged, not here.
    #[cfg(feature = "benchmark_counters")]
    {
        let counter_mode = CounterOutputMode::from_env();
        if counter_mode.emits_counter_summary() {
            for line in render_counter_summary(&snapshot) {
                saying::say!(line);
            }
        }
    }
}

// ---------------------------------------------------------------------------
//  No-op stubs (not(feature = "timers"))
// ---------------------------------------------------------------------------
//
// These stubs exist so that call sites using `#[cfg(feature = "timers")]` compile
// even when the feature is off. They are intentionally unused in default builds;
// the `allow(dead_code)` suppresses the expected warning.

/// No-op: start a collection scope. Does nothing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn start_benchmark_collection(_suppress_output: bool) {}

/// No-op: stop and collect. Returns an empty snapshot when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn stop_and_collect_benchmark_observations() -> BenchmarkObservationSnapshot {
    BenchmarkObservationSnapshot::default()
}

/// No-op: record a timing. Does nothing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn record_timing(_name: &str, _millis: f64) {}

/// No-op: record a counter. Does nothing when counter collection is inactive,
/// i.e. when either `timers` or `benchmark_counters` is off.
#[cfg(not(all(feature = "timers", feature = "benchmark_counters")))]
#[allow(dead_code)]
pub(crate) fn record_counter(_name: &str, _value: f64) {}

/// No-op: always returns true (output never suppressed when timers is off).
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn output_enabled() -> bool {
    true
}

/// No-op: record a pipeline timing. Does nothing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn record_pipeline_timing(_metric: &str, _duration: Duration) {}

/// No-op: record a labeled pipeline timing. Does nothing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn record_labeled_pipeline_timing(_metric: &str, _duration: Duration, _label: &str) {}

/// No-op: start a command timing scope. Does nothing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn start_command_timing() {}

/// No-op: print command timing summary. Does nothing when `timers` is off.
#[cfg(not(feature = "timers"))]
#[allow(dead_code)]
pub(crate) fn print_command_timing_summary() {}

// ---------------------------------------------------------------------------
//  Pipeline timer macros
// ---------------------------------------------------------------------------

/// Record a pipeline-stage timing with a stable metric name.
///
/// Usage:
/// ```ignore
/// let ast = pipeline_timer!("frontend.ast", build_ast()?);
/// ```
///
/// When `timers` is off, the macro expands to the wrapped expression, so the
/// metric name and `Instant` path are not evaluated or imported.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! pipeline_timer {
    ($metric:expr, $expression:expr) => {{
        let timing_start = std::time::Instant::now();
        let timing_result = $expression;
        $crate::timing::record_pipeline_timing($metric, timing_start.elapsed());
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! pipeline_timer {
    ($metric:expr, $expression:expr) => {{ $expression }};
}

/// Record a pipeline-stage timing with a stable metric name and a
/// human-readable label.
///
/// Usage:
/// ```ignore
/// let ast = labeled_pipeline_timer!("frontend.ast", "AST created in: ", build_ast()?);
/// ```
///
/// The human label is printed inline only in verbose mode. The stable
/// `MOTH_BENCH timing` line is emitted in bench or verbose mode.
///
/// When `timers` is off, the macro expands to the wrapped expression, so the
/// metric name, label, and `Instant` path are not evaluated or imported.
#[macro_export]
#[cfg(feature = "timers")]
macro_rules! labeled_pipeline_timer {
    ($metric:expr, $label:expr, $expression:expr) => {{
        let timing_start = std::time::Instant::now();
        let timing_result = $expression;
        $crate::timing::record_labeled_pipeline_timing($metric, timing_start.elapsed(), $label);
        timing_result
    }};
}

#[macro_export]
#[cfg(not(feature = "timers"))]
macro_rules! labeled_pipeline_timer {
    ($metric:expr, $label:expr, $expression:expr) => {{ $expression }};
}
