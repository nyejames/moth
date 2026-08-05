//! Enabled timer and benchmark-counter implementation.
//!
//! WHAT: owns collector state, output modes, observation types, summary
//!      rendering, command collection and RAII guards while the `timers`
//!      feature is active.
//! WHY:  compiling this module only when `timers` is selected keeps timer-only
//!       types, statics and renderer code out of no-timer binaries.
//!
//! Stage boundaries: this module owns timing and counter infrastructure only.
//! It must not import or depend on frontend, analysis, IR, or backend modules.
//! Compiler stages call into it through the `src/timing.rs` facade macros.

// Some collector-backed APIs can look unused when `timers` is active but no
// caller records a particular observation. This targeted allowance suppresses
// those expected dead-code warnings so `cargo check --features timers` stays
// quiet.
#![cfg_attr(feature = "timers", allow(dead_code))]

pub(crate) mod attribution;
pub(crate) mod collector;
pub(crate) mod mode;
pub(crate) mod render;
pub(crate) mod summary;

pub(crate) use attribution::{
    NO_TIMING_BOUNDARY, TimingBoundaryId, TimingBoundaryKind, TimingBoundaryRecord,
    TimingModuleAttribution, TimingModuleContext, TimingModuleKey, TimingModuleRecord,
};
pub(crate) use mode::TimerOutputMode;

use std::time::Duration;

// ---------------------------------------------------------------------------
//  Observation types
// ---------------------------------------------------------------------------

/// One named timing observation captured during a benchmark collection scope.
#[derive(Debug, Clone)]
pub(crate) struct TimingObservation {
    pub(crate) name: &'static str,
    pub(crate) duration: Duration,
    /// Optional attribution label preserved in raw observations for detailed
    /// tooling. The basic summary never renders it; slowest-module attribution
    /// comes from explicit registered module metadata.
    pub(crate) label: Option<String>,
    /// Compilation boundary for this observation, when attributed.
    pub(crate) boundary: Option<TimingBoundaryId>,
    /// Dense module key inside the boundary, when attributed.
    pub(crate) module: Option<TimingModuleKey>,
}

/// One named counter metric value captured during a benchmark collection scope.
///
/// Counters remain `f64` values; only timing observations carry `Duration`.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkObservationMetric {
    pub(crate) name: String,
    pub(crate) value: f64,
    /// Optional attribution label preserved in raw observations for detailed
    /// tooling. Never appears in stable `MOTH_BENCH counter` lines.
    pub(crate) label: Option<String>,
}

/// Snapshot of all observations captured in one collection scope.
///
/// `timings` is populated when the `timers` feature is active.
/// `counters` is only populated when both `timers` and `benchmark_counters`
/// are active, because counter storage reuses the same collector and is gated
/// behind `benchmark_counters`. `detailed_timers` alone no longer populates
/// counters.
#[derive(Debug, Clone, Default)]
pub(crate) struct BenchmarkObservationSnapshot {
    pub(crate) timings: Vec<TimingObservation>,
    pub(crate) counters: Vec<BenchmarkObservationMetric>,
    /// Registered compilation boundaries in deterministic registration order.
    pub(crate) boundaries: Vec<TimingBoundaryRecord>,
    /// Registered modules with their logical identities and source facts.
    pub(crate) modules: Vec<TimingModuleRecord>,
}

/// Aggregated view of repeated timing observations for summary output.
///
/// WHAT: combines multiple observations with the same stable metric name into
/// one total duration.
/// WHY: project-level timing summaries stay short even when later phases
/// record per-module metrics; raw labels and sample counts remain available
/// on the observations themselves.
#[derive(Debug, Clone, Default)]
pub(crate) struct TimingMetricSummary {
    pub(crate) total: Duration,
}

impl TimingMetricSummary {
    fn record(&mut self, duration: Duration) {
        self.total += duration;
    }
}

// ---------------------------------------------------------------------------
//  Public collection API
// ---------------------------------------------------------------------------

/// Start a benchmark collection scope.
///
/// When `suppress_output` is true, stdout is suppressed while observations
/// are still recorded in the collector. This is used by in-process frontend
/// benchmarks that read observations programmatically.
pub(crate) fn start_benchmark_collection(suppress_output: bool) {
    collector::start_collection(suppress_output);
}

/// Stop the current collection scope and return all captured observations.
///
/// Returns an empty snapshot when no scope was active.
pub(crate) fn stop_and_collect_benchmark_observations() -> BenchmarkObservationSnapshot {
    collector::stop_and_collect()
}

/// Record one timing observation in the active collection scope.
///
/// Called by `compiler_dev_logging::log_benchmark_timing` and by the
/// `pipeline_timer!` / `labeled_pipeline_timer!` macros.
pub(crate) fn record_timing(name: &'static str, duration: Duration) {
    collector::record_timing(name, duration);
}

/// Record one counter observation in the active collection scope.
///
/// Called by `compiler_dev_logging::log_benchmark_counter` and by the
/// Stage 0 discovery paths. Counter storage reuses the `timers` collector, so
/// this is only active when both `timers` and `benchmark_counters` are on.
/// `detailed_timers` alone no longer routes counters here.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn record_counter(name: &'static str, value: f64) {
    collector::record_counter(name, value);
}

/// Whether stdout output is currently allowed (not suppressed by an
/// in-process collection scope).
pub(crate) fn output_enabled() -> bool {
    collector::output_enabled()
}

/// The current timer output mode parsed from `MOTH_TIMERS`.
pub(crate) fn current_output_mode() -> TimerOutputMode {
    TimerOutputMode::from_env()
}

/// Emit one stable `MOTH_BENCH timing` line to stdout if the output mode
/// permits and output is not suppressed.
///
/// WHAT: prints a plain `MOTH_BENCH timing <metric>=<millis>ms` line that the
/// benchmark observation parser can grep without depending on human prose.
/// WHY: separating the stable metric line from colored human output lets
/// compiler logging change its prose without silently breaking attribution.
pub(crate) fn emit_bench_timing_line(name: &'static str, duration: Duration) {
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
pub(crate) fn record_pipeline_timing(metric: &'static str, duration: Duration) {
    record_timing(metric, duration);
    emit_bench_timing_line(metric, duration);
}

/// Register one compilation boundary in deterministic registration order.
///
/// Returns the dense boundary id for later observations and module
/// registration. With no active collection scope the call is a no-op that
/// returns a sentinel id; every subsequent attributed call is also dropped.
pub(crate) fn register_timing_boundary(
    kind: TimingBoundaryKind,
    display_name: String,
) -> TimingBoundaryId {
    collector::register_boundary(kind, display_name)
}

/// Register one module inside a boundary with its logical identity and source facts.
///
/// `module_index` is the boundary's dense graph `ModuleId`, so registration is
/// deterministic and independent of worker completion order. The logical
/// identity is composed from the boundary display name and the module's
/// portable logical path, never from an absolute filesystem path.
pub(crate) fn register_timing_module(
    boundary: TimingBoundaryId,
    module_index: u32,
    logical_module_path: &str,
    source_file_count: u64,
    source_byte_count: u64,
) -> TimingModuleKey {
    collector::register_module(
        boundary,
        module_index,
        logical_module_path,
        source_file_count,
        source_byte_count,
    )
}

/// Record a pipeline-stage timing with attribution context.
///
/// WHAT: like `record_pipeline_timing` but stores an optional human label plus
///      the compact boundary/module ids on the observation for the structured
///      summary.
/// WHY:  the basic summary needs registered metadata, never labels or paths,
///       to attribute boundary and slowest-module rows. The label never
///       appears in stable `MOTH_BENCH timing` lines.
pub(crate) fn record_pipeline_timing_attributed(
    metric: &'static str,
    duration: Duration,
    label: Option<&str>,
    context: TimingModuleContext,
) {
    collector::record_attributed_timing(metric, duration, label, context);
    emit_bench_timing_line(metric, duration);
}

/// Opaque start token for manually timed pipeline stages.
///
/// WHAT: stores an `Instant` only when the `timers` feature is active.
/// WHY: command/build orchestration sometimes needs to record a duration after
///      branching over error paths, where expression-wrapping macros would make
///      the control flow harder to read.
pub(crate) type PipelineTimingStart = std::time::Instant;

/// Start a manually recorded pipeline stage.
pub(crate) fn start_pipeline_timing() -> PipelineTimingStart {
    std::time::Instant::now()
}

/// Record a manually timed pipeline stage from a previously captured start token.
pub(crate) fn record_started_pipeline_timing(metric: &'static str, start: PipelineTimingStart) {
    record_pipeline_timing(metric, start.elapsed());
}

/// Record a manually timed pipeline stage with attribution context.
pub(crate) fn record_started_pipeline_timing_attributed(
    metric: &'static str,
    start: PipelineTimingStart,
    label: Option<&str>,
    context: TimingModuleContext,
) {
    record_pipeline_timing_attributed(metric, start.elapsed(), label, context);
}

/// RAII guard that records a pipeline-stage timing when dropped.
///
/// WHAT: captures a start instant on construction and records the elapsed
///      duration under the given metric name when the guard goes out of scope.
/// WHY:  backend orchestration has many early-return error paths; a Drop guard
///       ensures every stage is timed without scattering explicit record calls
///       before every `return Err`.
pub(crate) struct PipelineTimingGuard {
    metric: &'static str,
    start: PipelineTimingStart,
}

impl PipelineTimingGuard {
    /// Start timing a stage that will be recorded when the guard drops.
    pub(crate) fn new(metric: &'static str) -> Self {
        Self {
            metric,
            start: start_pipeline_timing(),
        }
    }
}

impl Drop for PipelineTimingGuard {
    fn drop(&mut self) {
        record_started_pipeline_timing(self.metric, self.start);
    }
}

/// Record a pipeline-stage timing with a human-readable label and emit the
/// stable bench line when appropriate.
///
/// Used by the `labeled_pipeline_timer!` macro. The human label is printed
/// inline only in verbose mode; the stable bench line depends on the output
/// mode and suppression flag.
pub(crate) fn record_labeled_pipeline_timing(
    metric: &'static str,
    duration: Duration,
    label: &str,
) {
    let mode = TimerOutputMode::from_env();

    if output_enabled() && mode.emits_human_prose() {
        saying::say!(label, Green #duration);
    }

    record_pipeline_timing(metric, duration);
}

// ---------------------------------------------------------------------------
//  Counter summary
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
//  Timing summary and command scope
// ---------------------------------------------------------------------------

/// Start a command-level timing collection scope.
///
/// WHAT: begins collecting timing observations for a single CLI command run so
///      that `print_command_timing_summary` can render a concise summary.
/// WHY:  the summary printed after `check` and `build` reads from this scope.
///       Call this at the top of a command handler, before any stage work begins.
pub(crate) fn start_command_timing() {
    start_benchmark_collection(false);
}

/// Stop the command timing scope and print the structured timing summary when
/// the output mode requests one.
///
/// WHAT: collects all recorded timings and prints the human-readable summary
///      after normal diagnostics/success output so timer prose never obscures
///      compiler messages.
/// WHY:  callers must print diagnostics first, then call this. In `Bench` mode
///      the summary is skipped (stable `MOTH_BENCH timing` lines were already
///      emitted inline). In `Silent` mode nothing is printed. The collection
///      scope is always stopped to clean up even when no summary is shown.
///      `succeeded` only changes the human title; stable metrics are unchanged.
pub(crate) fn print_command_timing_summary(succeeded: bool) {
    let mode = TimerOutputMode::from_env();

    let snapshot = stop_and_collect_benchmark_observations();

    if mode.emits_summary() {
        let command = if snapshot
            .timings
            .iter()
            .any(|observation| observation.name == "command.build.total")
        {
            summary::TimingCommandKind::Build
        } else {
            summary::TimingCommandKind::Check
        };
        let report = summary::build_timing_summary(&snapshot, command, succeeded);
        render::render_timing_summary_report(&report);
    }

    // Counter summary is owned by `benchmark_counters` and reuses the snapshot
    // just drained by the timing summary. It only prints when `MOTH_COUNTERS`
    // requests the concise summary view; the legacy full dump is printed inline
    // while counters are logged, not here.
    #[cfg(feature = "benchmark_counters")]
    {
        let counter_mode = crate::timing::CounterOutputMode::from_env();
        if counter_mode.emits_counter_summary() {
            for line in render_counter_summary(&snapshot) {
                saying::say!(line);
            }
        }
    }
}
