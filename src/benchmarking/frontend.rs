//! Frontend benchmark implementation.
//!
//! WHAT: measures the compiler frontend pipeline (Stage 0 through borrow
//! validation) for a single entry path, collecting total time and per-stage
//! timings when `timers` is enabled (counters additionally require
//! `benchmark_counters`).
//! WHY: avoids subprocess noise while reusing the exact same setup path as
//! `moth check`.

use std::path::PathBuf;
use std::time::Instant;

use crate::build_system::BuildProfile;
use crate::build_system::build::{BuildBootstrap, ProjectBuilder, bootstrap_project_build};
#[cfg(feature = "data_layout_memory_probe")]
use crate::build_system::create_project_modules::compiled_boundary::FrozenRenderRetentionMetrics;
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, compile_project_frontend_with_inputs,
};
use crate::build_system::path_validation::check_if_valid_path;
use crate::compiler_frontend::build_config::{
    BuildCommandLocation, BuildConfigInputEntry, BuildConfigInputSet, BuildConfigValueLocation,
    BuildInputName, PrimitiveBuildValue,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::diagnostic_severity::DiagnosticSeverity;
use crate::compiler_frontend::display_messages::format_terse_compiler_messages;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;

/// Build profile selector for frontend benchmarks.
///
/// WHAT: a narrow public selector that converts into the build-system `BuildProfile` at the
/// benchmark boundary without exposing internal compiler types in the benchmark API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendBenchmarkBuildProfile {
    Dev,
    Release,
}
/// One typed build-config input supplied to a frontend benchmark.
///
/// The benchmark adapter converts this value directly into the compiler-owned primitive carrier;
/// it never reparses benchmark text or waits for a source contract to infer a type.
#[derive(Debug, Clone, PartialEq)]
pub enum FrontendBenchmarkInputValue {
    String(String),
    Int(i32),
    Float(f64),
    Bool(bool),
    Char(char),
}

/// One named typed build-config input for a frontend benchmark.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontendBenchmarkInput {
    pub name: String,
    pub value: FrontendBenchmarkInputValue,
}

impl FrontendBenchmarkInput {
    pub fn new(name: impl Into<String>, value: FrontendBenchmarkInputValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// Input options for a single frontend benchmark run.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontendBenchmarkOptions {
    pub entry_path: PathBuf,
    pub build_profile: FrontendBenchmarkBuildProfile,
    pub build_config_inputs: Vec<FrontendBenchmarkInput>,
}

/// Whether a completed frontend benchmark compiled cleanly or produced user diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendBenchmarkOutcome {
    Success,
    Diagnosed,
}

/// Retained source and diagnostic storage observed at the frontend render boundary.
///
/// These values are layout counters, not allocator ownership attribution. They are populated only
/// with the `data_layout_memory_probe` feature; normal benchmark builds carry zeroes. Report
/// retention fields describe the returned message owner, while source-token and generic fields
/// preserve the command-scoped ledger so clean runs can still expose construction ownership. The
/// probe records them beside peak, live-report and after-report-drop allocator deltas so repeated
/// runs distinguish construction pressure from report-owner retention.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrontendBenchmarkRetention {
    pub source_snapshot_bytes: usize,
    pub extended_span_rows: usize,
    pub source_identity_slots: usize,
    pub diagnostic_records: usize,
    pub diagnostic_label_slots: usize,
    /// Distinct frozen identity contexts reachable from range rows or donor-only handles.
    pub retained_identity_contexts: usize,
    /// Distinct path-table owners reachable from the returned diagnostic report.
    pub path_table_count: usize,
    /// Actual retained `PathNode` rows across those deduplicated path tables.
    pub path_table_node_rows: usize,
    /// Backing vector capacity bytes for those deduplicated path tables.
    pub path_table_storage_bytes: usize,
    pub source_tokens_owners: usize,
    pub source_tokens_shape_bytes: usize,
    pub source_tokens_shape_capacity_bytes: usize,
    pub source_tokens_span_bytes: usize,
    pub source_tokens_span_capacity_bytes: usize,
    pub source_tokens_numeric_store_bytes: usize,
    pub source_tokens_numeric_store_capacity_bytes: usize,
    pub source_tokens_path_store_bytes: usize,
    pub source_tokens_path_store_capacity_bytes: usize,
    pub source_tokens_sequence_store_bytes: usize,
    pub source_tokens_sequence_store_capacity_bytes: usize,
    /// Cumulative builder shape/span capacities sampled at finish; not simultaneous live storage.
    pub transient_construction_buffer_bytes: usize,
    /// Largest individual builder buffer sample; a high-water observation, not an aggregate live
    /// allocation claim.
    pub transient_construction_peak_bytes: usize,
    pub cutover_adapter_bytes: usize,
    pub donor_identity_string_tables: usize,
    pub donor_identity_string_storage_bytes: usize,
    pub donor_identity_path_tables: usize,
    pub donor_identity_path_storage_bytes: usize,
    pub generic_source_tokens_owners: usize,
    pub generic_source_tokens_bytes: usize,
    /// Distinct generic source-owner bytes whose Arc is live at the final ledger snapshot.
    pub generic_source_tokens_live_bytes: usize,
    /// High-water distinct generic source-owner bytes observed at a ledger event.
    pub generic_source_tokens_peak_bytes: usize,
    pub requester_remap_count: usize,
    pub requester_remap_used_bytes: usize,
    pub requester_remap_capacity_bytes: usize,
    /// True when bounded probe bookkeeping could not observe every distinct owner.
    pub memory_ledger_incomplete: bool,

}
#[cfg(feature = "data_layout_memory_probe")]
impl From<FrozenRenderRetentionMetrics> for FrontendBenchmarkRetention {
    fn from(metrics: FrozenRenderRetentionMetrics) -> Self {
        Self {
            source_snapshot_bytes: metrics.source_snapshot_bytes,
            extended_span_rows: metrics.extended_span_rows,
            source_identity_slots: metrics.source_identity_slots,
            diagnostic_records: metrics.diagnostic_records,
            diagnostic_label_slots: metrics.diagnostic_label_slots,
            retained_identity_contexts: metrics.retained_identity_contexts,
            path_table_count: metrics.path_table_count,
            path_table_node_rows: metrics.path_table_node_rows,
            path_table_storage_bytes: metrics.path_table_storage_bytes,
            source_tokens_owners: metrics.source_tokens_owners,
            source_tokens_shape_bytes: metrics.source_tokens_shape_bytes,
            source_tokens_shape_capacity_bytes: metrics.source_tokens_shape_capacity_bytes,
            source_tokens_span_bytes: metrics.source_tokens_span_bytes,
            source_tokens_span_capacity_bytes: metrics.source_tokens_span_capacity_bytes,
            source_tokens_numeric_store_bytes: metrics.source_tokens_numeric_store_bytes,
            source_tokens_numeric_store_capacity_bytes: metrics
                .source_tokens_numeric_store_capacity_bytes,
            source_tokens_path_store_bytes: metrics.source_tokens_path_store_bytes,
            source_tokens_path_store_capacity_bytes: metrics
                .source_tokens_path_store_capacity_bytes,
            source_tokens_sequence_store_bytes: metrics.source_tokens_sequence_store_bytes,
            source_tokens_sequence_store_capacity_bytes: metrics
                .source_tokens_sequence_store_capacity_bytes,
            transient_construction_buffer_bytes: metrics.transient_construction_buffer_bytes,
            transient_construction_peak_bytes: metrics.transient_construction_peak_bytes,
            cutover_adapter_bytes: metrics.cutover_adapter_bytes,
            donor_identity_string_tables: metrics.donor_identity_string_tables,
            donor_identity_string_storage_bytes: metrics.donor_identity_string_storage_bytes,
            donor_identity_path_tables: metrics.donor_identity_path_tables,
            donor_identity_path_storage_bytes: metrics.donor_identity_path_storage_bytes,
            generic_source_tokens_owners: metrics.generic_source_tokens_owners,
            generic_source_tokens_bytes: metrics.generic_source_tokens_bytes,
            generic_source_tokens_live_bytes: metrics.generic_source_tokens_live_bytes,
            generic_source_tokens_peak_bytes: metrics.generic_source_tokens_peak_bytes,
            requester_remap_count: metrics.requester_remap_count,
            requester_remap_used_bytes: metrics.requester_remap_used_bytes,
            requester_remap_capacity_bytes: metrics.requester_remap_capacity_bytes,
            memory_ledger_incomplete: metrics.observation_incomplete,
        }
    }
}

/// Report produced by a completed frontend benchmark run.

#[derive(Debug, Clone)]
pub struct FrontendBenchmarkReport {
    /// Whether compilation completed cleanly or with user-facing diagnostics.
    pub outcome: FrontendBenchmarkOutcome,
    /// Number of error-severity user diagnostics.
    pub error_count: usize,
    /// Stable codes for the error-severity user diagnostics.
    pub diagnostic_codes: Vec<String>,
    /// Timing observation schema used by the stage list.
    pub timing_schema_version: u32,
    pub total_ms: f64,
    pub warning_count: usize,
    pub warning_codes: Vec<String>,
    pub retention: FrontendBenchmarkRetention,
    pub stages: Vec<FrontendBenchmarkStage>,
    pub counters: Vec<FrontendBenchmarkCounter>,
}

/// One named stage timing captured during frontend compilation.
#[derive(Debug, Clone)]
pub struct FrontendBenchmarkStage {
    pub name: String,
    pub duration_ms: f64,
}

/// One named counter value captured during frontend compilation.
#[derive(Debug, Clone)]
pub struct FrontendBenchmarkCounter {
    pub name: String,
    pub value: f64,
}

/// Error returned when a frontend benchmark fails.
///
/// The message is pre-rendered into a terse, multi-line string suitable for
/// direct display by xtask or other tooling. `kind` gives the failure a
/// structured identity so callers and tests can match the failure boundary
/// without reparsing rendered prose, and `diagnostic_codes` carries the stable
/// compiler diagnostic codes for compiler-backed failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendBenchmarkError {
    /// The boundary that rejected the benchmark.
    pub kind: FrontendBenchmarkFailureKind,
    /// Stable compiler diagnostic codes for path-validation, bootstrap and
    /// compilation failures. Empty for tooling-only failures.
    pub diagnostic_codes: Vec<String>,
    /// Terse pre-rendered message for direct display by xtask or tooling.
    pub message: String,
    /// Command-scoped ownership observations captured before this error escaped.
    pub retention: FrontendBenchmarkRetention,
}

/// Identifies the boundary that rejected a frontend benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendBenchmarkFailureKind {
    /// The raw benchmark timing session could not be acquired.
    TimingSession,
    /// The public benchmark options contained an invalid typed build-config input.
    BuildConfigInput,
    /// The entry path is not valid UTF-8.
    InvalidUtf8Path,
    /// The entry path failed validation (e.g. missing file).
    PathValidation,
    /// Project bootstrap failed before compilation.
    Bootstrap,
    /// The frontend pipeline emitted compiler diagnostics.
    Compilation,
}

impl std::fmt::Display for FrontendBenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[cfg(feature = "data_layout_memory_probe")]
fn snapshot_ledger_retention() -> FrontendBenchmarkRetention {
    FrontendBenchmarkRetention::from(FrozenRenderRetentionMetrics::from_ledger(
        crate::compiler_frontend::instrumentation::snapshot_memory_ledger(),
    ))
}

#[cfg(not(feature = "data_layout_memory_probe"))]
fn snapshot_ledger_retention() -> FrontendBenchmarkRetention {
    FrontendBenchmarkRetention::default()
}

fn frontend_benchmark_error_with_retention(
    kind: FrontendBenchmarkFailureKind,
    diagnostic_codes: Vec<String>,
    message: String,
    retention: FrontendBenchmarkRetention,
) -> FrontendBenchmarkError {
    FrontendBenchmarkError {
        kind,
        diagnostic_codes,
        message,
        retention,
    }
}

fn frontend_benchmark_error(
    kind: FrontendBenchmarkFailureKind,
    diagnostic_codes: Vec<String>,
    message: String,
) -> FrontendBenchmarkError {
    frontend_benchmark_error_with_retention(
        kind,
        diagnostic_codes,
        message,
        snapshot_ledger_retention(),
    )
}

fn frontend_benchmark_error_without_ledger(
    kind: FrontendBenchmarkFailureKind,
    diagnostic_codes: Vec<String>,
    message: String,
) -> FrontendBenchmarkError {
    frontend_benchmark_error_with_retention(
        kind,
        diagnostic_codes,
        message,
        FrontendBenchmarkRetention::default(),
    )
}

fn build_config_inputs_from_options(
    inputs: &[FrontendBenchmarkInput],
) -> Result<BuildConfigInputSet, FrontendBenchmarkError> {
    let mut typed_inputs = BuildConfigInputSet::new();

    for (index, input) in inputs.iter().enumerate() {
        let name = BuildInputName::new(&input.name).map_err(|_| {
            frontend_benchmark_error(
                FrontendBenchmarkFailureKind::BuildConfigInput,
                Vec::new(),
                format!(
                    "Frontend benchmark build-config input name '{}' is not lower_snake_case.",
                    input.name
                ),
            )
        })?;
        let value = match &input.value {
            FrontendBenchmarkInputValue::String(value) => {
                PrimitiveBuildValue::String(value.clone())
            }
            FrontendBenchmarkInputValue::Int(value) => PrimitiveBuildValue::Int(*value),
            FrontendBenchmarkInputValue::Float(value) => PrimitiveBuildValue::float(*value)
                .map_err(|_| {
                    frontend_benchmark_error(
                        FrontendBenchmarkFailureKind::BuildConfigInput,
                        Vec::new(),
                        format!(
                            "Frontend benchmark build-config input '{}' must use a finite Float.",
                            input.name
                        ),
                    )
                })?,
            FrontendBenchmarkInputValue::Bool(value) => PrimitiveBuildValue::Bool(*value),
            FrontendBenchmarkInputValue::Char(value) => PrimitiveBuildValue::Char(*value),
        };

        typed_inputs
            .insert(BuildConfigInputEntry::new(
                name,
                value,
                BuildConfigValueLocation::Command(BuildCommandLocation::new(index)),
            ))
            .map_err(|_| {
                frontend_benchmark_error(
                    FrontendBenchmarkFailureKind::BuildConfigInput,
                    Vec::new(),
                    format!(
                        "Frontend benchmark build-config input '{}' is repeated.",
                        input.name
                    ),
                )
            })?;
    }

    Ok(typed_inputs)
}
/// Run one frontend benchmark for the given entry path.
///
/// The ordinary API drops the completed render message owner before returning. The memory probe
/// uses the feature-gated owner-preserving API below so its allocator sample can distinguish live
/// report retention from the after-report-drop baseline.
pub fn run_frontend_benchmark(
    options: FrontendBenchmarkOptions,
) -> Result<FrontendBenchmarkReport, FrontendBenchmarkError> {
    run_frontend_benchmark_with_owner_internal(options).map(|(report, _)| report)
}
/// Warm the probe-only ownership ledger before its allocator baseline is sampled.
#[cfg(feature = "data_layout_memory_probe")]
pub fn prepare_frontend_memory_ledger() {
    crate::compiler_frontend::instrumentation::prepare_memory_ledger();
}

/// Completed benchmark report plus its live diagnostic/render owner.
///
/// This type exists only in the data-layout probe lane. Keeping the owner private prevents normal
/// callers from depending on compiler message internals while the value itself keeps that owner
/// alive until [`Self::into_report`] is called.
#[cfg(feature = "data_layout_memory_probe")]
#[derive(Debug)]
pub struct FrontendBenchmarkReportWithOwner {
    pub report: FrontendBenchmarkReport,
    owner: CompilerMessages,
}

#[cfg(feature = "data_layout_memory_probe")]
impl FrontendBenchmarkReportWithOwner {
    /// Drop the live report owner and return the public report value.
    pub fn into_report(self) -> FrontendBenchmarkReport {
        let Self { report, owner } = self;
        drop(owner);
        report
    }
}

/// Run a frontend benchmark while retaining the final diagnostic/render owner.
#[cfg(feature = "data_layout_memory_probe")]
pub fn run_frontend_benchmark_with_report_owner(
    options: FrontendBenchmarkOptions,
) -> Result<FrontendBenchmarkReportWithOwner, FrontendBenchmarkError> {
    run_frontend_benchmark_with_owner_internal(options)
        .map(|(report, owner)| FrontendBenchmarkReportWithOwner { report, owner })
}

fn run_frontend_benchmark_with_owner_internal(
    options: FrontendBenchmarkOptions,
) -> Result<(FrontendBenchmarkReport, CompilerMessages), FrontendBenchmarkError> {
    let start = Instant::now();

    // Acquire the raw session before resetting the command ledger. If another benchmark owns the
    // timing session, preserve that owner's ledger and return without observing or clearing it.
    #[cfg(feature = "timers")]
    let timing_session = crate::timing::start_raw_benchmark_collection(true).map_err(|error| {
        frontend_benchmark_error_without_ledger(
            FrontendBenchmarkFailureKind::TimingSession,
            Vec::new(),
            format!("Could not start frontend benchmark timing session: {error}"),
        )
    })?;
    // Reset at the benchmark command boundary, before bootstrap/config compilation. The
    // compilation module itself must not reset here because bootstrap owns measured preparation.
    crate::compiler_frontend::instrumentation::reset_memory_ledger();
    let requested_inputs = build_config_inputs_from_options(&options.build_config_inputs)?;

    let path = options
        .entry_path
        .to_str()
        .ok_or_else(|| {
            frontend_benchmark_error(
                FrontendBenchmarkFailureKind::InvalidUtf8Path,
                Vec::new(),
                format!(
                    "Frontend benchmark path is not valid UTF-8: {}",
                    options.entry_path.display()
                ),
            )
        })?;
    let normalized = if path.trim().is_empty() { "." } else { path };

    let valid_path = match check_if_valid_path(normalized) {
        Ok(path) => path,
        Err(error) => {
            let messages = CompilerMessages::from_error(error, StringTable::new());
            let mut diagnostic_codes = collect_diagnostic_codes(&messages);
            if messages.has_infrastructure_error() {
                diagnostic_codes.push("MOTH-INFRA-0001".to_owned());
            }
            return Err(frontend_benchmark_error(
                FrontendBenchmarkFailureKind::PathValidation,
                diagnostic_codes,
                format_compiler_messages(&messages),
            ));
        }
    };

    let project_builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
        mut project_source_files,
        build_config_inputs,
    } = match bootstrap_project_build(&project_builder, valid_path, &requested_inputs) {
        Ok(bootstrap) => bootstrap,
        Err(messages) => {
            let diagnostic_codes = collect_diagnostic_codes(&messages);
            return Err(frontend_benchmark_error(
                FrontendBenchmarkFailureKind::Bootstrap,
                diagnostic_codes,
                format_compiler_messages(&messages),
            ));
        }
    };

    let build_profile = match options.build_profile {
        FrontendBenchmarkBuildProfile::Release => BuildProfile::Release,
        FrontendBenchmarkBuildProfile::Dev => BuildProfile::Dev,
    };

    let (messages, retention, compilation_failed) = match compile_project_frontend_with_inputs(
        &mut config,
        build_profile,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    ) {
        Ok(frontend) => {
            #[cfg(feature = "data_layout_memory_probe")]
            let (messages, retention) = frontend
                .into_render_messages_with_frozen_identity_and_metrics(
                    &mut string_table,
                    project_source_files.take(),
                    None,
                )
                .map_err(|error| {
                    frontend_benchmark_error(
                        FrontendBenchmarkFailureKind::Compilation,
                        Vec::new(),
                        error.msg,
                    )
                })?;
            #[cfg(not(feature = "data_layout_memory_probe"))]
            let messages = frontend
                .into_render_messages_with_frozen_identity(
                    &mut string_table,
                    project_source_files.take(),
                    None,
                )
                .map_err(|error| {
                    frontend_benchmark_error(
                        FrontendBenchmarkFailureKind::Compilation,
                        Vec::new(),
                        error.msg,
                    )
                })?;
            #[cfg(feature = "data_layout_memory_probe")]
            let retention = FrontendBenchmarkRetention::from(retention);
            #[cfg(not(feature = "data_layout_memory_probe"))]
            let retention = FrontendBenchmarkRetention::default();
            (messages, retention, false)
        }
        Err(messages) => (messages, snapshot_ledger_retention(), true),
    };

    #[cfg(feature = "timers")]
    let snapshot = timing_session.finish();

    #[cfg(not(feature = "timers"))]
    let stages: Vec<FrontendBenchmarkStage> = Vec::new();

    #[cfg(not(all(feature = "timers", feature = "benchmark_counters")))]
    let counters: Vec<FrontendBenchmarkCounter> = Vec::new();

    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    let error_count = messages.error_count();
    let mut diagnostic_codes = collect_diagnostic_codes(&messages);
    let has_infrastructure_error = messages.has_infrastructure_error();
    if has_infrastructure_error {
        diagnostic_codes.push("MOTH-INFRA-0001".to_owned());
    }

    // User diagnostics are an expected benchmark outcome, but an infrastructure
    // failure or a failed compilation with no user error is still a runner
    // failure and must abort the benchmark.
    if has_infrastructure_error || (compilation_failed && error_count == 0) {
        return Err(frontend_benchmark_error_with_retention(
            FrontendBenchmarkFailureKind::Compilation,
            diagnostic_codes,
            format_compiler_messages(&messages),
            retention,
        ));
    }

    #[cfg(feature = "timers")]
    let stages = snapshot
        .timings
        .into_iter()
        .filter(|aggregate| aggregate.samples > 0)
        .map(|aggregate| FrontendBenchmarkStage {
            name: aggregate.metric.descriptor().stable_name.to_owned(),
            duration_ms: aggregate.total.as_secs_f64() * 1000.0,
        })
        .collect();

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counters = snapshot
        .counters
        .into_iter()
        .map(|metric| FrontendBenchmarkCounter {
            name: metric.name.to_owned(),
            value: metric.value,
        })
        .collect();

    let warning_count = messages.warning_count();
    let warning_codes = messages
        .warnings()
        .map(|warning| warning.kind.code().to_owned())
        .collect();
    let outcome = if error_count == 0 {
        FrontendBenchmarkOutcome::Success
    } else {
        FrontendBenchmarkOutcome::Diagnosed
    };
    let timing_schema_version = {
        #[cfg(feature = "timers")]
        {
            crate::benchmarking::TIMING_SCHEMA_VERSION
        }
        #[cfg(not(feature = "timers"))]
        {
            0
        }
    };

    Ok((
        FrontendBenchmarkReport {
            outcome,
            error_count,
            diagnostic_codes,
            timing_schema_version,
            total_ms,
            warning_count,
            warning_codes,
            retention,
            stages,
            counters,
        },
        messages,
    ))
}

fn format_compiler_messages(messages: &CompilerMessages) -> String {
    let mut lines = format_terse_compiler_messages(messages);
    if lines.is_empty() {
        lines.push(format!("{} error(s) found", messages.error_count()));
    }
    lines.join("\n")
}

/// Collects the stable diagnostic codes from the error diagnostics, preserving
/// order and multiplicity.
fn collect_diagnostic_codes(messages: &CompilerMessages) -> Vec<String> {
    messages
        .diagnostics()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .map(|diagnostic| diagnostic.kind.code().to_owned())
        .collect()
}
