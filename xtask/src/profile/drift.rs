//! Profile drift detection and reporting.
//!
//! WHAT: Compares the current profiling run against the latest previous
//! comparable record from `profile-runs.jsonl` and identifies significant
//! function, stage, and counter movements. Generates `profile-drift.md`
//! and a short drift section for `agent-summary.md`.
//!
//! WHY: Drift reports help agents identify where profiling time has shifted
//! between runs without re-parsing raw Samply profiles. Thresholds are
//! conservative to avoid surfacing sampling noise as significant changes.
//!
//! # What this module owns
//! - `DriftReport` and related structs for drift data
//! - `find_comparable_previous()` to locate the matching previous record
//! - `compute_drift()` to compare current and previous case data
//! - `format_drift_markdown()` for `profile-drift.md`
//! - `format_drift_summary_section()` for `agent-summary.md`
//!
//! # What this module does NOT own
//! - Profile history storage and retrieval (see `history.rs`)
//! - Profile JSON parsing or hotspot extraction (see `parse.rs`, `hotspots.rs`)
//! - Agent summaries and enriched per-case summaries (see `summary.rs`)

use crate::bench_types::BenchmarkMetric;

use super::history::{
    HistoryHotFunction, PROFILE_PROTOCOL_VERSION, ProfileHistoryRecord, StoredProfileHistoryRecord,
};

// ---------------------------------------------------------------------------
//  Drift thresholds
// ---------------------------------------------------------------------------

/// Minimum sample count for either current or previous to flag function drift.
const FUNCTION_MIN_SAMPLE_COUNT: usize = 300;

/// Minimum inclusive percent for either current or previous to flag function drift.
const FUNCTION_MIN_INCLUSIVE_PCT: f64 = 1.0;

/// Minimum absolute inclusive delta in percentage points to flag function drift.
const FUNCTION_MIN_DELTA_PCT: f64 = 2.0;

/// Minimum estimated inclusive milliseconds delta to flag function drift.
const FUNCTION_MIN_MS_DELTA: f64 = 20.0;

/// Minimum stage delta ratio (5%) to flag stage drift when absolute is also met.
const STAGE_MIN_DELTA_RATIO: f64 = 0.05;

/// Minimum absolute stage delta in milliseconds (10ms) when ratio is also met.
const STAGE_MIN_ABSOLUTE_MS: f64 = 10.0;

/// Minimum counter delta ratio (3%) to flag counter drift.
const COUNTER_MIN_DELTA_RATIO: f64 = 0.03;

/// Minimum absolute counter delta to avoid surfacing tiny movements.
const COUNTER_MIN_ABSOLUTE_DELTA: f64 = 5.0;

// ---------------------------------------------------------------------------
//  Data model
// ---------------------------------------------------------------------------

/// Complete drift report comparing current and previous profiling runs.
///
/// WHAT: Contains all significant movements found during drift comparison,
/// plus metadata about the comparison and counts of ignored noise.
///
/// WHY: A single struct makes the report generation and formatting
/// explicit and testable without threading many fields through functions.
#[derive(Debug)]
pub struct DriftReport {
    /// Previous run id, if a comparable record was found.
    pub previous_run_id: Option<String>,
    /// Case IDs whose source fingerprint changed (workload changed).
    pub workload_changed_case_ids: Vec<String>,
    /// Case IDs whose source matches but measurement fingerprint changed.
    pub measurement_changed_case_ids: Vec<String>,
    /// Significant function increases (inclusive pct grew).
    pub function_increases: Vec<FunctionDrift>,
    /// Significant function decreases (inclusive pct shrank).
    pub function_decreases: Vec<FunctionDrift>,
    /// Significant stage timing movements.
    pub stage_movements: Vec<StageDrift>,
    /// Significant counter movements.
    pub counter_movements: Vec<CounterDrift>,
    /// Number of function movements below threshold (noise).
    pub ignored_function_count: usize,
    /// Number of stage movements below threshold.
    pub ignored_stage_count: usize,
    /// Number of counter movements below threshold.
    pub ignored_counter_count: usize,
}

/// A significant function drift between current and previous runs.
#[derive(Debug, Clone)]
pub struct FunctionDrift {
    /// Authored case ID where the drift occurred.
    pub case_id: String,
    /// Function name that drifted.
    pub function_name: String,
    /// Current inclusive percentage.
    pub current_inclusive_pct: f64,
    /// Previous inclusive percentage.
    pub previous_inclusive_pct: f64,
    /// Delta in percentage points (current - previous).
    pub delta_pct: f64,
    /// Estimated inclusive milliseconds delta.
    pub estimated_ms_delta: f64,
    /// Owner bucket label.
    pub bucket_label: String,
    /// Whether this is a share-only drift (wall time did not move
    /// in the same direction as the function estimate).
    pub share_only: bool,
}

/// A significant stage timing drift between current and previous runs.
#[derive(Debug, Clone)]
pub struct StageDrift {
    /// Authored case ID where the drift occurred.
    pub case_id: String,
    /// Stage name that drifted.
    pub stage_name: String,
    /// Current stage timing in milliseconds.
    pub current_ms: f64,
    /// Previous stage timing in milliseconds.
    pub previous_ms: f64,
    /// Delta in milliseconds (current - previous).
    pub delta_ms: f64,
}

/// A significant counter drift between current and previous runs.
#[derive(Debug, Clone)]
pub struct CounterDrift {
    /// Authored case ID where the drift occurred.
    pub case_id: String,
    /// Counter name that drifted.
    pub counter_name: String,
    /// Current counter value.
    pub current_value: f64,
    /// Previous counter value.
    pub previous_value: f64,
    /// Delta percentage: (current - previous) / previous * 100.
    pub delta_pct: f64,
}

// ---------------------------------------------------------------------------
//  Public entry points
// ---------------------------------------------------------------------------

/// Find the latest previous record comparable to the current run.
///
/// WHAT: Scans history records from the end to find the most recent record
/// matching the same system UUID, filter mode, and optionally sample rate.
///
/// WHY: Drift comparison only makes sense between runs on the same system
/// with the same profiling configuration. This function locates that record
/// so `compute_drift()` can compare per-case data.
pub fn find_comparable_previous<'a>(
    records: &'a [StoredProfileHistoryRecord],
    system_uuid: &str,
    filter_mode: &str,
    sample_rate_hz: Option<f64>,
    current_run_id: &str,
) -> Option<&'a ProfileHistoryRecord> {
    records.iter().rev().find_map(|stored| {
        let StoredProfileHistoryRecord::Current(record) = stored else {
            // Legacy records are readable but never become drift baselines.
            return None;
        };

        (record.run_id != current_run_id
            && record.git_revision.is_clean_committed()
            && record.system_uuid == system_uuid
            && record.filter_mode == filter_mode
            && record.sample_rate_hz == sample_rate_hz
            && record.profile_protocol_version == PROFILE_PROTOCOL_VERSION)
            .then_some(record)
    })
}

/// Compute drift between the current profiling data and a previous record.
///
/// WHAT: Compares each current case against its matching previous case
/// (matched by case ID, command, and args). Applies function, stage,
/// and counter drift thresholds to identify significant movements.
///
/// WHY: Conservative thresholds avoid surfacing sampling noise. The
/// function thresholds require multiple conditions to be met simultaneously:
/// adequate sample count, meaningful percentage, significant delta, and
/// wall-time direction agreement (or explicit share-only marking).
pub fn compute_drift(
    current_cases: &[DriftCaseInput],
    previous: &ProfileHistoryRecord,
    current_wall_times: &std::collections::HashMap<String, f64>,
) -> DriftReport {
    let mut function_increases = Vec::new();
    let mut function_decreases = Vec::new();
    let mut stage_movements = Vec::new();
    let mut counter_movements = Vec::new();
    let mut ignored_function_count = 0usize;
    let mut ignored_stage_count = 0usize;
    let mut ignored_counter_count = 0usize;
    let mut workload_changed_case_ids = Vec::new();
    let mut measurement_changed_case_ids = Vec::new();

    for current in current_cases {
        // Match by stable authored case ID.
        let Some(previous_case) = previous
            .cases
            .iter()
            .find(|prev| prev.case_id == current.case_id)
        else {
            continue;
        };

        // Compare identity to classify the case. Current records always carry
        // a typed identity; legacy records never reach this comparison.
        if current.identity.source_fingerprint != previous_case.identity.source_fingerprint {
            workload_changed_case_ids.push(current.case_id.clone());
            continue;
        }
        if current.identity.measurement_fingerprint
            != previous_case.identity.measurement_fingerprint
        {
            measurement_changed_case_ids.push(current.case_id.clone());
            continue;
        }

        let current_wall = current_wall_times
            .get(&current.case_id)
            .copied()
            .unwrap_or(0.0);
        let previous_wall = previous_case.observation_wall_ms;
        let wall_delta = current_wall - previous_wall;

        // Compare functions.
        for current_func in &current.hot_functions {
            let Some(previous_func) = previous_case
                .hot_functions
                .iter()
                .find(|f| f.name == current_func.name)
            else {
                continue;
            };

            let delta_pct = current_func.inclusive_pct - previous_func.inclusive_pct;
            let estimated_ms_delta = (current_wall * current_func.inclusive_pct / 100.0)
                - (previous_wall * previous_func.inclusive_pct / 100.0);

            match classify_function_drift(
                current_func,
                previous_func,
                delta_pct,
                estimated_ms_delta,
                wall_delta,
            ) {
                FunctionDriftResult::Significant { share_only } => {
                    let drift = FunctionDrift {
                        case_id: current.case_id.clone(),
                        function_name: current_func.name.clone(),
                        current_inclusive_pct: current_func.inclusive_pct,
                        previous_inclusive_pct: previous_func.inclusive_pct,
                        delta_pct,
                        estimated_ms_delta,
                        bucket_label: current_func.bucket_label.clone(),
                        share_only,
                    };

                    if delta_pct > 0.0 {
                        function_increases.push(drift);
                    } else {
                        function_decreases.push(drift);
                    }
                }
                FunctionDriftResult::BelowThreshold => {
                    ignored_function_count += 1;
                }
            }
        }

        // Compare stage timings.
        for current_stage in &current.stage_timings {
            let Some(previous_stage) = previous_case
                .stage_timings
                .iter()
                .find(|s| s.name == current_stage.name)
            else {
                continue;
            };

            let delta_ms = current_stage.value - previous_stage.value;
            if is_significant_stage_drift(current_stage.value, previous_stage.value, delta_ms) {
                stage_movements.push(StageDrift {
                    case_id: current.case_id.clone(),
                    stage_name: current_stage.name.clone(),
                    current_ms: current_stage.value,
                    previous_ms: previous_stage.value,
                    delta_ms,
                });
            } else {
                ignored_stage_count += 1;
            }
        }

        // Compare counters.
        for current_counter in &current.counters {
            let Some(previous_counter) = previous_case
                .counters
                .iter()
                .find(|c| c.name == current_counter.name)
            else {
                continue;
            };

            let delta = current_counter.value - previous_counter.value;
            if is_significant_counter_drift(previous_counter.value, delta) {
                let delta_pct = if previous_counter.value != 0.0 {
                    (delta / previous_counter.value) * 100.0
                } else {
                    0.0
                };
                counter_movements.push(CounterDrift {
                    case_id: current.case_id.clone(),
                    counter_name: current_counter.name.clone(),
                    current_value: current_counter.value,
                    previous_value: previous_counter.value,
                    delta_pct,
                });
            } else {
                ignored_counter_count += 1;
            }
        }
    }

    // Sort by absolute delta descending.
    function_increases.sort_by(|a, b| b.delta_pct.total_cmp(&a.delta_pct));
    function_decreases.sort_by(|a, b| a.delta_pct.total_cmp(&b.delta_pct));
    stage_movements.sort_by(|a, b| b.delta_ms.abs().total_cmp(&a.delta_ms.abs()));
    counter_movements.sort_by(|a, b| b.delta_pct.abs().total_cmp(&a.delta_pct.abs()));

    DriftReport {
        previous_run_id: Some(previous.run_id.clone()),
        workload_changed_case_ids,
        measurement_changed_case_ids,
        function_increases,
        function_decreases,
        stage_movements,
        counter_movements,
        ignored_function_count,
        ignored_stage_count,
        ignored_counter_count,
    }
}

/// Build a `DriftReport` indicating no comparable previous record was found.
pub fn no_previous_drift_report() -> DriftReport {
    DriftReport {
        previous_run_id: None,
        workload_changed_case_ids: Vec::new(),
        measurement_changed_case_ids: Vec::new(),
        function_increases: Vec::new(),
        function_decreases: Vec::new(),
        stage_movements: Vec::new(),
        counter_movements: Vec::new(),
        ignored_function_count: 0,
        ignored_stage_count: 0,
        ignored_counter_count: 0,
    }
}

// ---------------------------------------------------------------------------
//  Drift case input (bridges current run data to drift comparison)
// ---------------------------------------------------------------------------

/// Input data for one case used by drift comparison.
///
/// WHAT: Combines the case identity, observation data, and hot function
/// data from the current profiling run into a single struct.
///
/// WHY: Drift comparison needs access to current case data in a shape
/// that matches `HistoryCaseRecord` without requiring the orchestrator
/// to build a full history record first.
#[derive(Debug)]
pub struct DriftCaseInput {
    /// Authored case ID from the typed benchmark manifest.
    pub case_id: String,
    /// Typed measurement identity from the current run.
    pub identity: crate::bench_types::BenchmarkMeasurementIdentity,
    /// The command executed (descriptive fact, not comparison authority).
    #[expect(dead_code)]
    pub command: String,
    /// Arguments passed to the command (descriptive fact, not comparison authority).
    #[expect(dead_code)]
    pub args: Vec<String>,
    /// Stage timings from the observation pass.
    pub stage_timings: Vec<BenchmarkMetric>,
    /// Counters from the observation pass.
    pub counters: Vec<BenchmarkMetric>,
    /// Hot functions from the current run.
    pub hot_functions: Vec<DriftHotFunction>,
}

/// A hot function entry for drift comparison.
///
/// WHAT: Stores the function name, bucket label, and inclusive percentage
/// needed for drift comparison.
///
/// WHY: Drift comparison only needs inclusive percentage and sample count;
/// self samples and other fields are not used in the comparison logic.
#[derive(Debug, Clone)]
pub struct DriftHotFunction {
    /// Function name.
    pub name: String,
    /// Owner bucket label.
    pub bucket_label: String,
    /// Inclusive sample count.
    pub inclusive_samples: f64,
    /// Inclusive percentage.
    pub inclusive_pct: f64,
}

// ---------------------------------------------------------------------------
//  Drift classification
// ---------------------------------------------------------------------------

/// Result of classifying a function drift.
enum FunctionDriftResult {
    /// The drift is significant and should be reported.
    Significant { share_only: bool },
    /// The drift is below threshold (noise).
    BelowThreshold,
}

/// Classify whether a function drift is significant.
///
/// WHAT: Applies the multi-condition function drift thresholds:
/// - sample count >= 300 (either current or previous)
/// - inclusive pct >= 1.0 (either current or previous)
/// - absolute inclusive delta >= 2.0 percentage points
/// - estimated inclusive ms delta >= 20ms
/// - wall time moved in the same direction, or report as share-only
///
/// WHY: All conditions must be met to avoid surfacing sampling noise.
/// The share-only classification catches cases where a function's share
/// grew but the overall case got faster (or vice versa).
fn classify_function_drift(
    current: &DriftHotFunction,
    previous: &HistoryHotFunction,
    delta_pct: f64,
    estimated_ms_delta: f64,
    wall_delta: f64,
) -> FunctionDriftResult {
    // Check sample count threshold.
    if current.inclusive_samples < FUNCTION_MIN_SAMPLE_COUNT as f64
        && previous.inclusive_samples < FUNCTION_MIN_SAMPLE_COUNT as f64
    {
        return FunctionDriftResult::BelowThreshold;
    }

    // Check inclusive percent threshold.
    if current.inclusive_pct < FUNCTION_MIN_INCLUSIVE_PCT
        && previous.inclusive_pct < FUNCTION_MIN_INCLUSIVE_PCT
    {
        return FunctionDriftResult::BelowThreshold;
    }

    // Check absolute delta threshold.
    if delta_pct.abs() < FUNCTION_MIN_DELTA_PCT {
        return FunctionDriftResult::BelowThreshold;
    }

    // Check estimated ms delta threshold.
    if estimated_ms_delta.abs() < FUNCTION_MIN_MS_DELTA {
        return FunctionDriftResult::BelowThreshold;
    }

    // Check wall time direction agreement.
    // If wall time and function estimate moved in the same direction,
    // this is a genuine drift. If they moved in opposite directions,
    // it's a share-only change (the function's share changed but the
    // overall case speed changed differently).
    let same_direction = (wall_delta >= 0.0 && estimated_ms_delta >= 0.0)
        || (wall_delta <= 0.0 && estimated_ms_delta <= 0.0);

    if same_direction {
        FunctionDriftResult::Significant { share_only: false }
    } else {
        // Share-only: report it but mark it.
        FunctionDriftResult::Significant { share_only: true }
    }
}

/// Check whether a stage drift is significant.
///
/// WHAT: Applies the stage drift threshold: either the existing benchmark
/// stage threshold (1.0ms or 5% of previous), or at least 5% and at least 10ms.
///
/// WHY: Two threshold paths catch both small absolute stages that changed
/// significantly in relative terms, and larger stages that moved enough
/// in absolute terms.
fn is_significant_stage_drift(_current_ms: f64, previous_ms: f64, delta_ms: f64) -> bool {
    // Path 1: existing benchmark stage threshold (1.0ms or 5% of previous).
    let benchmark_threshold = (previous_ms * 0.05).max(1.0);
    if delta_ms.abs() >= benchmark_threshold {
        return true;
    }

    // Path 2: at least 5% and at least 10ms.
    if previous_ms > 0.0 {
        let ratio = delta_ms.abs() / previous_ms;
        if ratio >= STAGE_MIN_DELTA_RATIO && delta_ms.abs() >= STAGE_MIN_ABSOLUTE_MS {
            return true;
        }
    }

    false
}

/// Check whether a counter drift is significant.
///
/// WHAT: Applies the counter drift threshold: at least 3% change AND
/// a meaningful absolute delta (at least 5.0).
///
/// WHY: Tiny absolute counter movements (e.g., 0.1 -> 0.103) should not
/// be surfaced even if the percentage is large. The absolute floor
/// prevents noise from dominating the report.
fn is_significant_counter_drift(previous: f64, delta: f64) -> bool {
    if previous == 0.0 {
        return delta.abs() >= COUNTER_MIN_ABSOLUTE_DELTA;
    }

    let ratio = (delta / previous).abs();
    ratio >= COUNTER_MIN_DELTA_RATIO && delta.abs() >= COUNTER_MIN_ABSOLUTE_DELTA
}

// ---------------------------------------------------------------------------
//  Report formatting
// ---------------------------------------------------------------------------

/// Format the complete drift report as Markdown.
///
/// WHAT: Generates `profile-drift.md` with sections for significant
/// function increases, decreases, stage movements, counter movements,
/// and ignored noise counts.
///
/// WHY: A dedicated markdown file gives agents a focused view of
/// profiling drift without parsing the full agent-summary.
pub fn format_drift_markdown(report: &DriftReport) -> String {
    let mut lines = Vec::new();

    lines.push("# Profiling drift".to_string());
    lines.push(String::new());

    match &report.previous_run_id {
        Some(run_id) => {
            lines.push(format!("Compared with: {}", run_id));
        }
        None => {
            lines.push("No previous comparable profile found.".to_string());
            lines.push(String::new());
            lines.push(
                "This is the first profile run with this configuration on this system.".to_string(),
            );
            return lines.join("\n");
        }
    }
    lines.push(String::new());

    // Identity changes.
    if !report.workload_changed_case_ids.is_empty() {
        lines.push("## Workload changed".to_string());
        lines.push(String::new());
        lines.push(format!(
            "Cases with changed source inputs (not comparable): {}",
            report.workload_changed_case_ids.join(", ")
        ));
        lines.push(String::new());
    }

    if !report.measurement_changed_case_ids.is_empty() {
        lines.push("## Measurement changed".to_string());
        lines.push(String::new());
        lines.push(format!(
            "Cases with changed runner or protocol (not comparable): {}",
            report.measurement_changed_case_ids.join(", ")
        ));
        lines.push(String::new());
    }

    // Significant increases.
    lines.push("## Significant increases".to_string());
    lines.push(String::new());
    if report.function_increases.is_empty() {
        lines.push("None.".to_string());
    } else {
        lines.push(
            "| Case | Function | Inclusive | Previous | Delta | Est. ms delta | Bucket |"
                .to_string(),
        );
        lines.push("|---|---|---:|---:|---:|---:|---|".to_string());
        for drift in &report.function_increases {
            lines.push(format_function_drift_row(drift));
        }
    }
    lines.push(String::new());

    // Significant decreases.
    lines.push("## Significant decreases".to_string());
    lines.push(String::new());
    if report.function_decreases.is_empty() {
        lines.push("None.".to_string());
    } else {
        lines.push(
            "| Case | Function | Inclusive | Previous | Delta | Est. ms delta | Bucket |"
                .to_string(),
        );
        lines.push("|---|---|---:|---:|---:|---:|---|".to_string());
        for drift in &report.function_decreases {
            lines.push(format_function_drift_row(drift));
        }
    }
    lines.push(String::new());

    // Stage movement.
    lines.push("## Significant stage movement".to_string());
    lines.push(String::new());
    if report.stage_movements.is_empty() {
        lines.push("None.".to_string());
    } else {
        lines.push("| Case | Stage | Current | Previous | Delta |".to_string());
        lines.push("|---|---|---:|---:|---:|".to_string());
        for drift in &report.stage_movements {
            lines.push(format_stage_drift_row(drift));
        }
    }
    lines.push(String::new());

    // Counter movement.
    lines.push("## Significant counter movement".to_string());
    lines.push(String::new());
    if report.counter_movements.is_empty() {
        lines.push("None.".to_string());
    } else {
        lines.push("| Case | Counter | Current | Previous | Delta % |".to_string());
        lines.push("|---|---|---:|---:|---:|".to_string());
        for drift in &report.counter_movements {
            lines.push(format_counter_drift_row(drift));
        }
    }
    lines.push(String::new());

    // Ignored noise.
    let total_ignored =
        report.ignored_function_count + report.ignored_stage_count + report.ignored_counter_count;

    if total_ignored > 0 {
        lines.push("## Ignored noise".to_string());
        lines.push(String::new());
        if report.ignored_function_count > 0 {
            lines.push(format!(
                "- {} function movements below threshold.",
                report.ignored_function_count
            ));
        }
        if report.ignored_stage_count > 0 {
            lines.push(format!(
                "- {} stage movements below threshold.",
                report.ignored_stage_count
            ));
        }
        if report.ignored_counter_count > 0 {
            lines.push(format!(
                "- {} counter movements below threshold.",
                report.ignored_counter_count
            ));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Format a short drift section for inclusion in `agent-summary.md`.
///
/// WHAT: Produces a concise section with the most important drift items:
/// top function increases, top function decreases, top stage movements,
/// and a noise summary.
///
/// WHY: The agent summary should not duplicate the full drift report;
/// this section highlights the most actionable items.
pub fn format_drift_summary_section(report: &DriftReport) -> String {
    let mut lines = Vec::new();

    lines.push("## Drift".to_string());
    lines.push(String::new());

    match &report.previous_run_id {
        Some(run_id) => {
            lines.push(format!("Compared with: {}", run_id));
        }
        None => {
            lines.push("No previous comparable profile.".to_string());
            return lines.join("\n");
        }
    }
    lines.push(String::new());

    // Identity changes.
    if !report.workload_changed_case_ids.is_empty() {
        lines.push(format!(
            "Workload changed: {} (new baseline required)",
            report.workload_changed_case_ids.join(", ")
        ));
        lines.push(String::new());
    }

    if !report.measurement_changed_case_ids.is_empty() {
        lines.push(format!(
            "Measurement changed: {} (new baseline required)",
            report.measurement_changed_case_ids.join(", ")
        ));
        lines.push(String::new());
    }

    // Top function increases (up to 3).
    let increases: Vec<_> = report.function_increases.iter().take(3).collect();
    if !increases.is_empty() {
        lines.push("### Increases".to_string());
        lines.push(String::new());
        for drift in &increases {
            let share_marker = if drift.share_only {
                " (share-only)"
            } else {
                ""
            };
            lines.push(format!(
                "- `{}` in {}: {:.1}% → {:.1}% (+{:.1}pp, ~{:.0}ms){}",
                truncate_name(&drift.function_name, 60),
                drift.case_id,
                drift.previous_inclusive_pct,
                drift.current_inclusive_pct,
                drift.delta_pct,
                drift.estimated_ms_delta,
                share_marker,
            ));
        }
        lines.push(String::new());
    }

    // Top function decreases (up to 3).
    let decreases: Vec<_> = report.function_decreases.iter().take(3).collect();
    if !decreases.is_empty() {
        lines.push("### Decreases".to_string());
        lines.push(String::new());
        for drift in &decreases {
            let share_marker = if drift.share_only {
                " (share-only)"
            } else {
                ""
            };
            lines.push(format!(
                "- `{}` in {}: {:.1}% → {:.1}% ({:.1}pp, ~{:.0}ms){}",
                truncate_name(&drift.function_name, 60),
                drift.case_id,
                drift.previous_inclusive_pct,
                drift.current_inclusive_pct,
                drift.delta_pct,
                drift.estimated_ms_delta,
                share_marker,
            ));
        }
        lines.push(String::new());
    }

    // Top stage movements (up to 3).
    let stages: Vec<_> = report.stage_movements.iter().take(3).collect();
    if !stages.is_empty() {
        lines.push("### Stage movement".to_string());
        lines.push(String::new());
        for drift in &stages {
            lines.push(format!(
                "- `{}` in {}: {:.0}ms → {:.0}ms ({:+.0}ms)",
                drift.stage_name,
                drift.case_id,
                drift.previous_ms,
                drift.current_ms,
                drift.delta_ms,
            ));
        }
        lines.push(String::new());
    }

    // Noise summary.
    let total_ignored =
        report.ignored_function_count + report.ignored_stage_count + report.ignored_counter_count;
    if total_ignored > 0 {
        lines.push(format!("_{} movements below threshold._", total_ignored));
        lines.push(String::new());
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
//  Formatting helpers
// ---------------------------------------------------------------------------

/// Format a function drift as a markdown table row.
fn format_function_drift_row(drift: &FunctionDrift) -> String {
    let share_marker = if drift.share_only { " (share)" } else { "" };
    format!(
        "| {} | `{}` | {:.1}% | {:.1}% | {:+.1}pp | ~{:+.0}ms | {}{} |",
        drift.case_id,
        truncate_name(&drift.function_name, 48),
        drift.current_inclusive_pct,
        drift.previous_inclusive_pct,
        drift.delta_pct,
        drift.estimated_ms_delta,
        drift.bucket_label,
        share_marker,
    )
}

/// Format a stage drift as a markdown table row.
fn format_stage_drift_row(drift: &StageDrift) -> String {
    format!(
        "| {} | {} | {:.0}ms | {:.0}ms | {:+.0}ms |",
        drift.case_id, drift.stage_name, drift.current_ms, drift.previous_ms, drift.delta_ms,
    )
}

/// Format a counter drift as a markdown table row.
fn format_counter_drift_row(drift: &CounterDrift) -> String {
    format!(
        "| {} | {} | {:.0} | {:.0} | {:+.1}% |",
        drift.case_id,
        drift.counter_name,
        drift.current_value,
        drift.previous_value,
        drift.delta_pct,
    )
}

/// Truncate a function name to a maximum length with `...`.
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len.saturating_sub(3)])
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "drift_tests.rs"]
mod tests;
