//! Benchmark domain types - Core data structures for benchmark results
//!
//! This module provides named structs for benchmark measurements, statistics,
//! and comparisons. It replaces tuple-heavy APIs with explicit types that
//! document the meaning of each field.

use crate::benchmark_manifest::BenchmarkRunner;
use std::fmt::{Display, Formatter};
use std::num::NonZeroUsize;

/// Identity of the benchmark measurement and workload-comparison protocol.
///
/// Increment this only when measurement methodology or workload fingerprint
/// semantics change enough to make direct comparisons invalid.
pub const BENCHMARK_PROTOCOL_VERSION: u32 = 2;

/// Selects which manifest cases proceed to measured benchmark iterations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkSelection {
    Full,
    Quick,
}

/// Controls whether a successful benchmark run may enter persistence paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkRecording {
    Record,
    ReadOnly,
}

/// Typed policy for the measured part of one mandatory-preflight benchmark run.
///
/// Preflight deliberately does not appear here. Every normal benchmark
/// orchestrator runs it exactly once before applying this measured-run policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BenchmarkRunPolicy {
    measured_iterations: NonZeroUsize,
    selection: BenchmarkSelection,
    recording: BenchmarkRecording,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkRunPolicyError {
    ZeroMeasuredIterations,
    RecordingRequiresFullSelection,
}

impl Display for BenchmarkRunPolicyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroMeasuredIterations => {
                write!(formatter, "measured benchmark iterations must be nonzero")
            }
            Self::RecordingRequiresFullSelection => {
                write!(
                    formatter,
                    "recording benchmark runs require full case selection"
                )
            }
        }
    }
}

impl std::error::Error for BenchmarkRunPolicyError {}

impl BenchmarkRunPolicy {
    pub(crate) fn new(
        measured_iterations: usize,
        selection: BenchmarkSelection,
        recording: BenchmarkRecording,
    ) -> Result<Self, BenchmarkRunPolicyError> {
        let measured_iterations = NonZeroUsize::new(measured_iterations)
            .ok_or(BenchmarkRunPolicyError::ZeroMeasuredIterations)?;

        if recording == BenchmarkRecording::Record && selection != BenchmarkSelection::Full {
            return Err(BenchmarkRunPolicyError::RecordingRequiresFullSelection);
        }

        Ok(Self {
            measured_iterations,
            selection,
            recording,
        })
    }

    pub(crate) fn measured_iterations(self) -> NonZeroUsize {
        self.measured_iterations
    }

    pub(crate) fn selection(self) -> BenchmarkSelection {
        self.selection
    }

    pub(crate) fn recording(self) -> BenchmarkRecording {
        self.recording
    }

    pub(crate) fn selects_case(self, quick: bool) -> bool {
        match self.selection {
            BenchmarkSelection::Full => true,
            BenchmarkSelection::Quick => quick,
        }
    }
}

/// Distinguishes the two benchmark suite kinds so local history and summaries
/// do not accidentally compare incompatible metrics.
///
/// WHAT: CLI subprocess wall-clock time vs in-process frontend stage time.
/// WHY: Prevents a frontend refactor from being compared against CLI spawn
///      overhead, and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkSuiteKind {
    /// End-to-end CLI benchmark measuring subprocess wall-clock time.
    EndToEndCli,
    /// In-process frontend benchmark measuring compiler stage timings.
    FrontendPhases,
}

impl BenchmarkSuiteKind {
    /// Parse a persisted suite kind from local JSONL records.
    pub fn from_persisted_name(name: &str) -> Option<Self> {
        match name {
            "end_to_end_cli" => Some(BenchmarkSuiteKind::EndToEndCli),
            "frontend_phases" => Some(BenchmarkSuiteKind::FrontendPhases),
            _ => None,
        }
    }

    /// Persistent string used in local JSONL records.
    pub fn persisted_name(&self) -> &'static str {
        match self {
            BenchmarkSuiteKind::EndToEndCli => "end_to_end_cli",
            BenchmarkSuiteKind::FrontendPhases => "frontend_phases",
        }
    }

    /// Human-readable display label used in summaries and terminal output.
    pub fn display_label(&self) -> &'static str {
        match self {
            BenchmarkSuiteKind::EndToEndCli => "End-to-end CLI",
            BenchmarkSuiteKind::FrontendPhases => "Frontend phases",
        }
    }

    /// Primary metric name for this suite kind.
    pub fn primary_metric_name(&self) -> &'static str {
        match self {
            BenchmarkSuiteKind::EndToEndCli => "wall_time_ms",
            BenchmarkSuiteKind::FrontendPhases => "frontend_total_ms",
        }
    }
}

/// Typed identity for one benchmark case measurement.
///
/// Combines the workload identity with the case measurement fingerprint so
/// comparisons can distinguish source changes from runner/expectation changes
/// without loose optional strings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkMeasurementIdentity {
    /// Authored workload identity from the manifest.
    pub workload_id: String,
    /// Source workload fingerprint, absent only for adapted legacy history.
    pub source_fingerprint: String,
    /// Case measurement fingerprint covering source, protocol, runner and expectation.
    pub measurement_fingerprint: String,
}

/// A single benchmark case result after measured iterations.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkCaseResult {
    /// Authored stable benchmark case identity.
    pub case_id: String,
    /// Typed measurement identity, absent only for adapted legacy history.
    pub identity: Option<BenchmarkMeasurementIdentity>,
    /// Public grouping used by summaries to give absolute context.
    pub group_name: String,
    /// Typed runner declaration, including CLI command/profile and authored args.
    pub runner: BenchmarkRunner,
    /// Mean duration in milliseconds across measured iterations.
    pub mean_ms: f64,
    /// Median duration in milliseconds across measured iterations.
    pub median_ms: f64,
    /// Standard deviation in milliseconds across measured iterations.
    pub stddev_ms: f64,
    /// Local-only detailed timer and counter observations parsed from stdout.
    pub observations: BenchmarkCaseObservations,
}

/// One named timing or counter value captured from detailed compiler output.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkMetric {
    pub name: String,
    pub value: f64,
}

/// Local-only detailed observations for one benchmark case.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BenchmarkCaseObservations {
    pub stage_timings: Vec<BenchmarkMetric>,
    pub counters: Vec<BenchmarkMetric>,
}

/// Aggregated statistics for one benchmark group.
///
/// Groups are deliberately simple summary buckets, not compiler-stage
/// categories. They make public benchmark output easier to compare without
/// committing per-case timing tables.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkGroupStats {
    /// Public group label.
    pub group_name: String,
    /// Number of cases in this group.
    pub case_count: usize,
    /// Average of per-case means in milliseconds.
    pub average_ms: f64,
}

/// Aggregated statistics for the entire benchmark suite.
///
/// WHAT: Summarises per-case means into a single average and case spread.
/// WHY: The spread is across heterogeneous benchmark cases, not statistical
/// measurement noise from repeated runs of the same case.
#[derive(Debug, Clone)]
pub struct SuiteStats {
    /// Average of per-case means in milliseconds.
    pub average_ms: f64,
    /// Standard deviation across per-case means in milliseconds.
    pub case_spread_ms: f64,
}

impl SuiteStats {
    /// Compute suite stats from a list of per-case results.
    ///
    /// WHAT: Extracts per-case means and computes the suite average plus
    /// cross-case spread.
    /// WHY: Naming the spread accurately prevents summary code from treating
    /// unrelated benchmark variety as repeated-measurement uncertainty.
    pub fn from_case_results(cases: &[BenchmarkCaseResult]) -> Self {
        let means: Vec<f64> = cases.iter().map(|c| c.mean_ms).collect();
        let average_ms = calculate_mean(&means);
        let case_spread_ms = calculate_stddev(&means, average_ms);

        Self {
            average_ms,
            case_spread_ms,
        }
    }
}

/// Classification of benchmark change relative to a previous run
///
/// WHAT: Named interpretation of whether a run changed meaningfully.
/// WHY: Run-level classification must be derived from case classifications so
/// mixed faster/slower movement cannot collapse into a misleading no-change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkChangeKind {
    /// No previous overlapping benchmark cases exist
    Baseline,
    /// Previous comparison exists but no overlapping case exceeded its threshold
    NoMeasurableChange,
    /// At least one case improved and no cases regressed
    Faster,
    /// At least one case regressed and no cases improved
    Slower,
    /// At least one case improved and at least one case regressed
    Mixed,
}

/// Classification of a single overlapping benchmark case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkCaseChangeKind {
    /// The case delta stayed within its local measured-variation threshold.
    NoMeasurableChange,
    /// The current case mean is meaningfully lower than the previous mean.
    Faster,
    /// The current case mean is meaningfully higher than the previous mean.
    Slower,
}

/// Named rough-threshold configuration for benchmark comparisons.
///
/// WHAT: Defines the absolute and relative floors used to classify case
/// and stage movement as meaningful.
/// WHY: Prevents magic constants from drifting across the comparison and
/// display code, and makes the threshold policy explicit and testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BenchmarkThresholds {
    pub minimum_case_delta_ms: f64,
    pub minimum_case_delta_ratio: f64,
    pub stddev_multiplier: f64,
    pub minimum_stage_delta_ms: f64,
    pub minimum_stage_delta_ratio: f64,
}

impl BenchmarkThresholds {
    /// Default thresholds calibrated for rough compiler-development sanity checks.
    ///
    /// WHAT: Catches obvious movement without over-reporting subprocess noise.
    /// WHY: These values were chosen after observing typical CLI and frontend
    /// benchmark variation on development hardware.
    pub const DEFAULT: Self = Self {
        minimum_case_delta_ms: 2.0,
        minimum_case_delta_ratio: 0.03,
        stddev_multiplier: 2.0,
        minimum_stage_delta_ms: 1.0,
        minimum_stage_delta_ratio: 0.05,
    };
}

/// Comparison between one current case and its previous counterpart.
#[derive(Debug, Clone)]
pub struct BenchmarkCaseComparison {
    pub case_id: String,
    pub group_name: String,
    pub previous_mean_ms: f64,
    pub current_mean_ms: f64,
    pub delta_ms: f64,
    pub threshold_ms: f64,
    pub change_kind: BenchmarkCaseChangeKind,
    pub observations: BenchmarkObservationComparison,
}

/// Comparison between one current stage timing and its previous counterpart.
///
/// WHAT: Retains both previous and current absolute values so local
/// diagnostics and future debug tooling can inspect them.
/// WHY: The public summary only uses `delta_ms`, but local JSONL history
/// and future per-stage drill-down will need the raw values.
#[derive(Debug, Clone)]
pub struct BenchmarkStageComparison {
    pub stage_name: String,
    pub previous_ms: f64,
    pub current_ms: f64,
    pub delta_ms: f64,
    pub change_kind: BenchmarkCaseChangeKind,
}

/// Comparison between current and previous observations for one case.
#[derive(Debug, Clone)]
pub struct BenchmarkObservationComparison {
    pub stage_comparisons: Vec<BenchmarkStageComparison>,
}

/// Aggregated stage movement across all overlapping cases in a comparison.
#[derive(Debug, Clone)]
pub struct BenchmarkStageMovement {
    pub stage_name: String,
    pub total_delta_ms: f64,
    pub case_count: usize,
    pub faster_count: usize,
    pub slower_count: usize,
}

/// Aggregated comparison counts for a current benchmark group.
#[derive(Debug, Clone)]
pub struct BenchmarkGroupComparison {
    pub group_name: String,
    pub previous_average_ms: Option<f64>,
    pub current_average_ms: f64,
    pub delta_ms: Option<f64>,
    pub faster_count: usize,
    pub slower_count: usize,
    pub unchanged_count: usize,
}

/// Comparison between a current benchmark run and a previous one
///
/// WHAT: Computes per-case deltas, group counts, and the run-level
/// classification between two runs.
/// WHY: The displayed run result should report mixed movement honestly instead
/// of using spread across unrelated cases as a noise threshold.
#[derive(Debug, Clone)]
pub struct BenchmarkComparison {
    /// Overall mean change in milliseconds, or None if no previous run
    pub overall_mean_delta_ms: Option<f64>,
    /// Named classification of the change.
    pub change_kind: BenchmarkChangeKind,
    /// Number of current cases with matching stable IDs and identical identity.
    pub compared_case_count: usize,
    /// Number of current cases.
    pub current_case_count: usize,
    /// Number of previous cases.
    pub previous_case_count: usize,
    /// Number of overlapping cases classified as faster.
    pub faster_case_count: usize,
    /// Number of overlapping cases classified as slower.
    pub slower_case_count: usize,
    /// Number of overlapping cases classified as unchanged.
    pub unchanged_case_count: usize,
    /// True when cases were added or removed between the two runs.
    pub case_set_changed: bool,
    /// Number of matching stable IDs whose source workload changed.
    pub workload_changed_case_count: usize,
    /// Source-changed stable IDs in current manifest order.
    pub workload_changed_case_ids: Vec<String>,
    /// Number of matching stable IDs whose source matches but measurement changed.
    pub measurement_changed_case_count: usize,
    /// Measurement-changed stable IDs in current manifest order.
    pub measurement_changed_case_ids: Vec<String>,
    /// Per-case comparisons for overlapping cases.
    pub cases: Vec<BenchmarkCaseComparison>,
    /// Per-current-group comparison counts and average movement.
    pub groups: Vec<BenchmarkGroupComparison>,
    /// Average of current per-case means in milliseconds.
    ///
    /// Lets the baseline summary line show absolute timing when no comparable
    /// previous run exists (first run or a different thread identity).
    pub current_suite_average_ms: f64,
}

impl BenchmarkComparison {
    /// Compare current case results against an optional previous set.
    ///
    /// WHAT: Finds overlapping cases by stable ID, excludes changed workloads,
    /// classifies comparable cases, then derives run and group summaries.
    /// WHY: Per-case classification catches mixed movement and single-case
    /// regressions that a suite-level average can hide.
    ///
    /// If there are no overlapping cases with the previous run, or if no
    /// previous run is provided, the comparison reports baseline.
    pub fn new(current: &[BenchmarkCaseResult], previous: Option<&[BenchmarkCaseResult]>) -> Self {
        Self::new_with_thresholds(current, previous, &BenchmarkThresholds::DEFAULT)
    }

    /// Compare current case results with explicit threshold policy.
    ///
    /// WHAT: Lets tests and future xtask-only callers exercise threshold
    /// behavior without changing the normal benchmark command defaults.
    /// WHY: Threshold tuning is part of the benchmark domain, so exact policy
    /// tests should not depend on hidden constants.
    pub fn new_with_thresholds(
        current: &[BenchmarkCaseResult],
        previous: Option<&[BenchmarkCaseResult]>,
        thresholds: &BenchmarkThresholds,
    ) -> Self {
        let Some(previous_cases) = previous else {
            return Self::baseline(
                current.len(),
                0,
                false,
                Vec::new(),
                Vec::new(),
                current,
                None,
            );
        };

        let matched_cases = match_cases(current, previous_cases, thresholds);
        let cases = matched_cases.comparable;
        let workload_changed_case_ids = matched_cases.workload_changed_case_ids;
        let measurement_changed_case_ids = matched_cases.measurement_changed_case_ids;
        let workload_changed_case_count = workload_changed_case_ids.len();
        let measurement_changed_case_count = measurement_changed_case_ids.len();
        let case_set_changed = case_ids_changed(current, previous_cases);

        if cases.is_empty() {
            return Self::baseline(
                current.len(),
                previous_cases.len(),
                case_set_changed,
                workload_changed_case_ids,
                measurement_changed_case_ids,
                current,
                Some(previous_cases),
            );
        }
        debug_assert!(cases.iter().all(|case| {
            !case.case_id.is_empty()
                && case.previous_mean_ms.is_finite()
                && case.current_mean_ms.is_finite()
        }));

        let faster_case_count = cases
            .iter()
            .filter(|case| case.change_kind == BenchmarkCaseChangeKind::Faster)
            .count();
        let slower_case_count = cases
            .iter()
            .filter(|case| case.change_kind == BenchmarkCaseChangeKind::Slower)
            .count();
        let unchanged_case_count = cases
            .iter()
            .filter(|case| case.change_kind == BenchmarkCaseChangeKind::NoMeasurableChange)
            .count();

        let change_kind = classify_run_change(faster_case_count, slower_case_count);
        let deltas: Vec<f64> = cases.iter().map(|case| case.delta_ms).collect();
        let overall_mean_delta_ms = Some(calculate_mean(&deltas));
        let compared_case_count = cases.len();
        let current_case_count = current.len();
        let previous_case_count = previous_cases.len();
        let groups = compare_groups(current, previous_cases, &cases);

        let comparison = Self {
            current_suite_average_ms: Self::mean_of_case_means(current),
            overall_mean_delta_ms,
            change_kind,
            compared_case_count,
            current_case_count,
            previous_case_count,
            faster_case_count,
            slower_case_count,
            unchanged_case_count,
            case_set_changed,
            workload_changed_case_count,
            workload_changed_case_ids,
            measurement_changed_case_count,
            measurement_changed_case_ids,
            cases,
            groups,
        };

        comparison.debug_assert_consistent();
        comparison
    }

    fn baseline(
        current_case_count: usize,
        previous_case_count: usize,
        case_set_changed: bool,
        workload_changed_case_ids: Vec<String>,
        measurement_changed_case_ids: Vec<String>,
        current: &[BenchmarkCaseResult],
        previous: Option<&[BenchmarkCaseResult]>,
    ) -> Self {
        let groups = previous
            .map(|previous_cases| compare_groups(current, previous_cases, &[]))
            .unwrap_or_else(|| baseline_groups(current));

        let comparison = Self {
            overall_mean_delta_ms: None,
            change_kind: BenchmarkChangeKind::Baseline,
            compared_case_count: 0,
            current_case_count,
            previous_case_count,
            faster_case_count: 0,
            slower_case_count: 0,
            unchanged_case_count: 0,
            case_set_changed,
            workload_changed_case_count: workload_changed_case_ids.len(),
            workload_changed_case_ids,
            measurement_changed_case_count: measurement_changed_case_ids.len(),
            measurement_changed_case_ids,
            cases: Vec::new(),
            groups,
            current_suite_average_ms: Self::mean_of_case_means(current),
        };

        comparison.debug_assert_consistent();
        comparison
    }

    /// Mean of per-case mean timings for the current run.
    ///
    /// Avoids allocating a temporary Vec so the baseline summary line can
    /// show absolute timing without reusing `calculate_mean`.
    fn mean_of_case_means(cases: &[BenchmarkCaseResult]) -> f64 {
        if cases.is_empty() {
            0.0
        } else {
            cases.iter().map(|case| case.mean_ms).sum::<f64>() / cases.len() as f64
        }
    }

    /// Check comparison aggregates while keeping detailed fields live.
    ///
    /// WHAT: Validates that the public summary counts still agree with the
    /// retained per-case and per-group detail.
    /// WHY: The benchmark model intentionally stores detail beyond the terse
    /// monthly summary so future diagnostics can inspect cases without
    /// reparsing history.
    fn debug_assert_consistent(&self) {
        let case_count = self.cases.len();
        let classified_case_count =
            self.faster_case_count + self.slower_case_count + self.unchanged_case_count;

        debug_assert_eq!(case_count, self.compared_case_count);
        debug_assert_eq!(classified_case_count, self.compared_case_count);

        for case in &self.cases {
            debug_assert!(!case.case_id.trim().is_empty());
            debug_assert!(!case.group_name.trim().is_empty());
            debug_assert!(case.previous_mean_ms.is_finite());
            debug_assert!(case.current_mean_ms.is_finite());
            debug_assert!(case.delta_ms.is_finite());
            debug_assert!(case.threshold_ms >= 0.0 && case.threshold_ms.is_finite());

            for stage in &case.observations.stage_comparisons {
                debug_assert!(!stage.stage_name.trim().is_empty());
                debug_assert!(stage.previous_ms.is_finite());
                debug_assert!(stage.current_ms.is_finite());
                debug_assert!(stage.delta_ms.is_finite());
            }
        }

        let grouped_case_count: usize = self
            .groups
            .iter()
            .map(|group| {
                debug_assert!(!group.group_name.trim().is_empty());
                debug_assert!(group.current_average_ms.is_finite());

                if let Some(previous_average_ms) = group.previous_average_ms {
                    debug_assert!(previous_average_ms.is_finite());
                }

                if let Some(delta_ms) = group.delta_ms {
                    debug_assert!(delta_ms.is_finite());
                }

                group.faster_count + group.slower_count + group.unchanged_count
            })
            .sum();

        debug_assert_eq!(grouped_case_count, self.compared_case_count);
        debug_assert_eq!(
            self.workload_changed_case_ids.len(),
            self.workload_changed_case_count
        );
        debug_assert_eq!(
            self.measurement_changed_case_ids.len(),
            self.measurement_changed_case_count
        );
    }

    /// Format the run-entry summary line for display in monthly summaries.
    ///
    /// Returns:
    /// - "**baseline**; N cases, avg ~Xms" for the first run on a system.
    ///
    ///   Includes the absolute current suite average so the user can see
    ///   whether the run was fast or slow even when no comparable previous
    ///   record exists (first run or a different thread identity).
    /// - "no measurable change: avg +0ms; N/N cases" when all shared cases
    ///   stayed within their thresholds.
    /// - terse faster/slower/mixed/case-set-changed lines otherwise.
    pub fn format_run_change_line(&self) -> String {
        let timing_line = if self.case_set_changed {
            self.format_case_set_changed_line()
        } else {
            match self.change_kind {
                BenchmarkChangeKind::Baseline => {
                    if self.workload_changed_case_count > 0 {
                        "no comparable unchanged workloads".to_string()
                    } else {
                        format!(
                            "**baseline**; {} cases, avg ~{}ms",
                            self.current_case_count,
                            self.current_suite_average_ms.round() as i64
                        )
                    }
                }
                BenchmarkChangeKind::NoMeasurableChange => {
                    format!(
                        "no measurable change: avg {}; {}/{} cases",
                        format_signed_ms(self.overall_mean_delta_ms.unwrap_or(0.0)),
                        self.compared_case_count,
                        self.current_case_count
                    )
                }
                BenchmarkChangeKind::Faster | BenchmarkChangeKind::Slower => {
                    format!(
                        "**{} avg**; {} faster, {} slower; {}/{} cases",
                        format_signed_ms(self.overall_mean_delta_ms.unwrap_or(0.0)),
                        self.faster_case_count,
                        self.slower_case_count,
                        self.compared_case_count,
                        self.current_case_count
                    )
                }
                BenchmarkChangeKind::Mixed => format!(
                    "mixed: avg {}; {} faster, {} slower; {}/{} cases",
                    format_signed_ms(self.overall_mean_delta_ms.unwrap_or(0.0)),
                    self.faster_case_count,
                    self.slower_case_count,
                    self.compared_case_count,
                    self.current_case_count
                ),
            }
        };

        if self.workload_changed_case_count == 0 && self.measurement_changed_case_count == 0 {
            timing_line
        } else {
            let mut segments = Vec::new();
            if self.workload_changed_case_count > 0 {
                segments.push(self.format_workload_changed_segment());
            }
            if self.measurement_changed_case_count > 0 {
                segments.push(self.format_measurement_changed_segment());
            }
            format!("{timing_line}; {}", segments.join("; "))
        }
    }

    fn format_case_set_changed_line(&self) -> String {
        if let Some(delta) = self.overall_mean_delta_ms {
            let shared_denominator = self.current_case_count.max(self.previous_case_count);

            format!(
                "case set changed: avg {} on {}/{} shared cases; {} slower, {} faster",
                format_signed_ms(delta),
                self.compared_case_count,
                shared_denominator,
                self.slower_case_count,
                self.faster_case_count
            )
        } else {
            let comparison_state = if self.workload_changed_case_count > 0 {
                "no comparable unchanged workloads"
            } else {
                "no shared cases"
            };

            format!(
                "case set changed: {comparison_state}; {} current, {} previous",
                self.current_case_count, self.previous_case_count
            )
        }
    }

    fn format_workload_changed_segment(&self) -> String {
        let case_label = if self.workload_changed_case_count == 1 {
            "case"
        } else {
            "cases"
        };

        format!(
            "workload changed: {} {} ({})",
            self.workload_changed_case_count,
            case_label,
            self.workload_changed_case_ids.join(", ")
        )
    }

    fn format_measurement_changed_segment(&self) -> String {
        let case_label = if self.measurement_changed_case_count == 1 {
            "case"
        } else {
            "cases"
        };

        format!(
            "measurement changed: {} {} ({})",
            self.measurement_changed_case_count,
            case_label,
            self.measurement_changed_case_ids.join(", ")
        )
    }

    /// Compare a quick current subset against only the same previous IDs.
    ///
    /// Intentional selection differences do not become removals, while a
    /// newly added quick case still remains a case-set change.
    pub fn for_quick_subset(
        current: &[BenchmarkCaseResult],
        previous: Option<&[BenchmarkCaseResult]>,
    ) -> Self {
        let Some(previous) = previous else {
            return Self::new(current, None);
        };
        let current_ids: std::collections::HashSet<&str> =
            current.iter().map(|case| case.case_id.as_str()).collect();
        let filtered_previous: Vec<BenchmarkCaseResult> = previous
            .iter()
            .filter(|case| current_ids.contains(case.case_id.as_str()))
            .cloned()
            .collect();

        Self::new(current, Some(&filtered_previous))
    }
}

struct MatchedCases {
    comparable: Vec<BenchmarkCaseComparison>,
    workload_changed_case_ids: Vec<String>,
    measurement_changed_case_ids: Vec<String>,
}

fn match_cases(
    current: &[BenchmarkCaseResult],
    previous: &[BenchmarkCaseResult],
    thresholds: &BenchmarkThresholds,
) -> MatchedCases {
    let mut cases = Vec::new();
    let mut workload_changed_case_ids = Vec::new();
    let mut measurement_changed_case_ids = Vec::new();

    for current_case in current {
        let Some(previous_case) = previous
            .iter()
            .find(|case| case.case_id == current_case.case_id)
        else {
            continue;
        };

        let (current_identity, previous_identity) = match (
            current_case.identity.as_ref(),
            previous_case.identity.as_ref(),
        ) {
            (Some(current), Some(previous)) => (current, previous),
            _ => {
                // Adapted legacy records lack identity; skip them as
                // incomparable rather than silently matching on missing data.
                continue;
            }
        };

        if current_identity.source_fingerprint != previous_identity.source_fingerprint {
            workload_changed_case_ids.push(current_case.case_id.clone());
            continue;
        }

        if current_identity.measurement_fingerprint != previous_identity.measurement_fingerprint {
            measurement_changed_case_ids.push(current_case.case_id.clone());
            continue;
        }

        let delta_ms = current_case.mean_ms - previous_case.mean_ms;
        let threshold_ms = case_threshold_ms(current_case, previous_case, thresholds);
        let change_kind = classify_case_change(delta_ms, threshold_ms);
        let observations = compare_observations(
            &current_case.observations,
            &previous_case.observations,
            thresholds,
        );

        cases.push(BenchmarkCaseComparison {
            case_id: current_case.case_id.clone(),
            group_name: current_case.group_name.clone(),
            previous_mean_ms: previous_case.mean_ms,
            current_mean_ms: current_case.mean_ms,
            delta_ms,
            threshold_ms,
            change_kind,
            observations,
        });
    }

    MatchedCases {
        comparable: cases,
        workload_changed_case_ids,
        measurement_changed_case_ids,
    }
}

fn case_ids_changed(current: &[BenchmarkCaseResult], previous: &[BenchmarkCaseResult]) -> bool {
    current.iter().any(|current_case| {
        !previous
            .iter()
            .any(|previous_case| previous_case.case_id == current_case.case_id)
    }) || previous.iter().any(|previous_case| {
        !current
            .iter()
            .any(|current_case| current_case.case_id == previous_case.case_id)
    })
}

fn case_threshold_ms(
    current: &BenchmarkCaseResult,
    previous: &BenchmarkCaseResult,
    thresholds: &BenchmarkThresholds,
) -> f64 {
    let combined_stddev = (current.stddev_ms.powi(2) + previous.stddev_ms.powi(2)).sqrt();
    let stddev_component = combined_stddev * thresholds.stddev_multiplier;
    let ratio_component = previous.mean_ms * thresholds.minimum_case_delta_ratio;

    thresholds
        .minimum_case_delta_ms
        .max(ratio_component)
        .max(stddev_component)
}

fn classify_case_change(delta_ms: f64, threshold_ms: f64) -> BenchmarkCaseChangeKind {
    if delta_ms.abs() <= threshold_ms {
        BenchmarkCaseChangeKind::NoMeasurableChange
    } else if delta_ms < -threshold_ms {
        BenchmarkCaseChangeKind::Faster
    } else {
        BenchmarkCaseChangeKind::Slower
    }
}

fn classify_run_change(faster_count: usize, slower_count: usize) -> BenchmarkChangeKind {
    match (faster_count > 0, slower_count > 0) {
        (true, true) => BenchmarkChangeKind::Mixed,
        (true, false) => BenchmarkChangeKind::Faster,
        (false, true) => BenchmarkChangeKind::Slower,
        (false, false) => BenchmarkChangeKind::NoMeasurableChange,
    }
}

fn baseline_groups(current: &[BenchmarkCaseResult]) -> Vec<BenchmarkGroupComparison> {
    calculate_group_stats(current)
        .into_iter()
        .map(|group| BenchmarkGroupComparison {
            group_name: group.group_name,
            previous_average_ms: None,
            current_average_ms: group.average_ms,
            delta_ms: None,
            faster_count: 0,
            slower_count: 0,
            unchanged_count: 0,
        })
        .collect()
}

fn compare_groups(
    current: &[BenchmarkCaseResult],
    _previous: &[BenchmarkCaseResult],
    compared_cases: &[BenchmarkCaseComparison],
) -> Vec<BenchmarkGroupComparison> {
    let current_groups = calculate_group_stats(current);

    current_groups
        .into_iter()
        .map(|current_group| {
            let comparable_in_group: Vec<&BenchmarkCaseComparison> = compared_cases
                .iter()
                .filter(|case| case.group_name == current_group.group_name)
                .collect();
            let previous_average_ms =
                average_comparison_values(&comparable_in_group, |case| case.previous_mean_ms);
            let comparable_current_average_ms =
                average_comparison_values(&comparable_in_group, |case| case.current_mean_ms);
            let current_average_ms =
                comparable_current_average_ms.unwrap_or(current_group.average_ms);
            let delta_ms = previous_average_ms
                .zip(comparable_current_average_ms)
                .map(|(previous_average, current_average)| current_average - previous_average);
            let faster_count = count_group_cases(
                compared_cases,
                &current_group.group_name,
                BenchmarkCaseChangeKind::Faster,
            );
            let slower_count = count_group_cases(
                compared_cases,
                &current_group.group_name,
                BenchmarkCaseChangeKind::Slower,
            );
            let unchanged_count = count_group_cases(
                compared_cases,
                &current_group.group_name,
                BenchmarkCaseChangeKind::NoMeasurableChange,
            );

            BenchmarkGroupComparison {
                group_name: current_group.group_name,
                previous_average_ms,
                current_average_ms,
                delta_ms,
                faster_count,
                slower_count,
                unchanged_count,
            }
        })
        .collect()
}

fn average_comparison_values(
    cases: &[&BenchmarkCaseComparison],
    value: impl Fn(&BenchmarkCaseComparison) -> f64,
) -> Option<f64> {
    if cases.is_empty() {
        None
    } else {
        Some(cases.iter().map(|case| value(case)).sum::<f64>() / cases.len() as f64)
    }
}

fn count_group_cases(
    cases: &[BenchmarkCaseComparison],
    group_name: &str,
    change_kind: BenchmarkCaseChangeKind,
) -> usize {
    cases
        .iter()
        .filter(|case| case.group_name == group_name && case.change_kind == change_kind)
        .count()
}

fn format_signed_ms(value: f64) -> String {
    let rounded = value.round() as i64;
    if rounded > 0 {
        format!("+{}ms", rounded)
    } else {
        format!("{}ms", rounded)
    }
}

/// Describes the local system that ran the benchmark
///
/// WHAT: Privacy-safe identity for a single machine/clone
/// WHY: Allows per-system tracking without exposing machine-derived identifiers
#[derive(Debug, Clone)]
pub struct BenchmarkSystem {
    /// Stable private UUID for this clone (local-only)
    pub system_uuid: String,
    /// Short public hex identifier shown in summaries
    pub public_system_id: String,
    /// Human-readable display name (e.g., "macOS M1")
    pub display_name: String,
}

/// A complete recorded benchmark run
///
/// WHAT: All data from one full benchmark execution
/// WHY: Stored in local raw history and used to generate summaries
#[derive(Debug, Clone)]
pub struct BenchmarkRun {
    /// Timestamp when the run started
    pub timestamp: crate::bench_time::BenchmarkTimestamp,
    /// Benchmark measurement/workload protocol persisted with this run.
    pub benchmark_protocol_version: u32,
    /// Git commit and dirty state, when each fact is available.
    pub git_revision: GitRevision,
    /// System that performed the run
    pub system: BenchmarkSystem,
    /// Which benchmark suite kind this run belongs to
    pub suite_kind: BenchmarkSuiteKind,
    /// Per-case results
    pub cases: Vec<BenchmarkCaseResult>,
    /// Aggregated statistics per public benchmark group.
    pub groups: Vec<BenchmarkGroupStats>,
    /// Aggregated suite statistics
    pub suite: SuiteStats,
    /// Number of warmup runs performed before measurement
    pub warmup_runs: usize,
    /// Number of measured iterations used for each case
    pub measured_iterations: usize,
    /// Effective RAYON_NUM_THREADS setting: None for default threads, Some(n) for a fixed count.
    pub thread_count: Option<u32>,
}

/// Best-effort source revision metadata for one benchmark run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitRevision {
    pub commit: Option<String>,
    #[serde(rename = "git_dirty")]
    pub dirty: Option<bool>,
}

impl GitRevision {
    /// Whether this revision is exactly clean and committed.
    ///
    /// Comparable recorded runs require a captured commit and no dirty state.
    pub(crate) fn is_clean_committed(&self) -> bool {
        self.commit
            .as_deref()
            .is_some_and(|commit| !commit.is_empty())
            && self.dirty == Some(false)
    }
}

/// Calculate mean of a slice of values
pub fn calculate_mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

/// Calculate median of a slice of values.
///
/// WHAT: Sorts a local copy and returns the middle value, or the average of
/// the two middle values for even-sized inputs.
/// WHY: Benchmark summaries will keep mean as the primary public average, but
/// median is useful local raw data for judging noisy subprocess measurements.
pub fn calculate_median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut sorted_values = values.to_vec();
    sorted_values.sort_by(|left, right| left.total_cmp(right));

    let middle = sorted_values.len() / 2;
    if sorted_values.len() % 2 == 1 {
        sorted_values[middle]
    } else {
        let left = sorted_values[middle - 1];
        let right = sorted_values[middle];
        (left + right) / 2.0
    }
}

/// Calculate standard deviation of a slice of values
pub fn calculate_stddev(values: &[f64], mean: f64) -> f64 {
    if values.len() <= 1 {
        return 0.0;
    }
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    variance.sqrt()
}

/// Calculate average timing per benchmark group.
///
/// WHAT: Groups case means by their public group label and returns stable
/// summary records.
/// WHY: Later summary phases need absolute group context without duplicating
/// grouping and ordering logic in render code.
pub fn calculate_group_stats(cases: &[BenchmarkCaseResult]) -> Vec<BenchmarkGroupStats> {
    let mut groups: Vec<BenchmarkGroupStatsBuilder> = Vec::new();

    for case in cases {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group.group_name == case.group_name)
        {
            group.case_means.push(case.mean_ms);
        } else {
            groups.push(BenchmarkGroupStatsBuilder {
                group_name: case.group_name.clone(),
                case_means: vec![case.mean_ms],
            });
        }
    }

    let mut stats: Vec<BenchmarkGroupStats> = groups
        .into_iter()
        .map(|group| {
            let average_ms = calculate_mean(&group.case_means);

            BenchmarkGroupStats {
                group_name: group.group_name,
                case_count: group.case_means.len(),
                average_ms,
            }
        })
        .collect();

    stats.sort_by(|left, right| {
        group_sort_key(&left.group_name).cmp(&group_sort_key(&right.group_name))
    });

    stats
}

struct BenchmarkGroupStatsBuilder {
    group_name: String,
    case_means: Vec<f64>,
}

fn group_sort_key(group_name: &str) -> (usize, &str) {
    match group_name {
        "core" => (0, group_name),
        "docs" => (1, group_name),
        "stress" => (2, group_name),
        "module" => (3, group_name),
        "borrow" => (4, group_name),
        _ => (usize::MAX, group_name),
    }
}

/// Compare current and previous observations for overlapping stage timings.
///
/// WHAT: Finds stage metrics present in both current and previous observations,
/// calculates deltas, classifies them against rough thresholds, and sorts by
/// absolute delta descending.
/// WHY: Stage attribution helps identify which compiler phases changed.
pub fn compare_observations(
    current: &BenchmarkCaseObservations,
    previous: &BenchmarkCaseObservations,
    thresholds: &BenchmarkThresholds,
) -> BenchmarkObservationComparison {
    let mut stage_comparisons = Vec::new();

    for current_metric in &current.stage_timings {
        let Some(previous_metric) = previous
            .stage_timings
            .iter()
            .find(|metric| metric.name == current_metric.name)
        else {
            continue;
        };

        let delta_ms = current_metric.value - previous_metric.value;
        let threshold_ms =
            stage_threshold_ms(previous_metric.value, current_metric.value, thresholds);
        let change_kind = classify_stage_change(delta_ms, threshold_ms);

        stage_comparisons.push(BenchmarkStageComparison {
            stage_name: current_metric.name.clone(),
            previous_ms: previous_metric.value,
            current_ms: current_metric.value,
            delta_ms,
            change_kind,
        });
    }

    stage_comparisons.sort_by(|left, right| right.delta_ms.abs().total_cmp(&left.delta_ms.abs()));

    BenchmarkObservationComparison { stage_comparisons }
}

/// Rough threshold for whether a stage delta is meaningful.
///
/// WHAT: Uses the configured larger of an absolute floor or a percentage of
/// the previous stage time.
/// WHY: Tiny fast stages need an absolute jitter guard; slower stages need a
/// percentage guard to avoid over-reporting stable small movement.
fn stage_threshold_ms(previous_ms: f64, _current_ms: f64, thresholds: &BenchmarkThresholds) -> f64 {
    let ratio_threshold = previous_ms * thresholds.minimum_stage_delta_ratio;
    ratio_threshold.max(thresholds.minimum_stage_delta_ms)
}

fn classify_stage_change(delta_ms: f64, threshold_ms: f64) -> BenchmarkCaseChangeKind {
    if delta_ms < -threshold_ms {
        BenchmarkCaseChangeKind::Faster
    } else if delta_ms > threshold_ms {
        BenchmarkCaseChangeKind::Slower
    } else {
        BenchmarkCaseChangeKind::NoMeasurableChange
    }
}

/// Aggregate stage movement across all overlapping cases in a comparison.
///
/// WHAT: Sums per-case stage deltas and counts faster/slower classifications.
/// WHY: Run-level stage attribution makes it obvious whether a change
/// affected AST, headers, HIR, or borrow checking.
pub fn calculate_stage_movement(comparison: &BenchmarkComparison) -> Vec<BenchmarkStageMovement> {
    let mut movement_by_stage: std::collections::BTreeMap<String, BenchmarkStageMovement> =
        std::collections::BTreeMap::new();

    for case in &comparison.cases {
        for stage in &case.observations.stage_comparisons {
            let movement = movement_by_stage.entry(stage.stage_name.clone()).or_insert(
                BenchmarkStageMovement {
                    stage_name: stage.stage_name.clone(),
                    total_delta_ms: 0.0,
                    case_count: 0,
                    faster_count: 0,
                    slower_count: 0,
                },
            );

            movement.total_delta_ms += stage.delta_ms;
            movement.case_count += 1;
            match stage.change_kind {
                BenchmarkCaseChangeKind::Faster => movement.faster_count += 1,
                BenchmarkCaseChangeKind::Slower => movement.slower_count += 1,
                BenchmarkCaseChangeKind::NoMeasurableChange => {}
            }
        }
    }

    let mut movements: Vec<BenchmarkStageMovement> = movement_by_stage.into_values().collect();
    movements.sort_by(|left, right| {
        right
            .total_delta_ms
            .abs()
            .total_cmp(&left.total_delta_ms.abs())
    });

    movements
}

/// Convert a raw stage metric name to a short friendly label.
pub fn friendly_stage_label(stage_name: &str) -> &str {
    match stage_name {
        // Dotted top-level command-phase metrics (timers feature).
        "command.check.path_validation" => "check path",
        "command.check.builder_construction" => "check builder",
        "command.check.bootstrap" => "check bootstrap",
        "command.check.compile_project_frontend" => "check frontend",
        "command.check.message_rendering" => "check render",
        "command.check.total" => "check total",
        "command.build.output_write" => "build output",
        "command.build.total" => "build total",
        "build_project.path_validation" => "build path",
        "build_project.bootstrap" => "build bootstrap",
        "build_project.compile_project_frontend" => "build frontend",
        "build_project.backend" => "backend",
        "build_project.total" => "build project",
        // Dotted bootstrap and output metrics.
        "bootstrap.total" => "bootstrap",
        "bootstrap.config_init" => "config init",
        "bootstrap.symbol_preseed" => "symbol preseed",
        "bootstrap.backend_libraries" => "backend libraries",
        "bootstrap.style_directives" => "style directives",
        "bootstrap.load_project_config" => "load config",
        "bootstrap.backend_config_validate" => "backend config",
        "output.write_total" => "output write",
        "output.prepare_cleanup" => "output prep",
        "output.create_root" => "output root",
        "output.emit_files_total" => "emit files",
        "output.emit_file" => "emit file",
        "output.finalize_cleanup" => "output cleanup",
        // Dotted Stage 0 and config metrics.
        "config.load_total" => "config load",
        "config.file_exists_check" => "config exists",
        "config.parse_project_config_file" => "config parse",
        "config.parse.total" => "config parse",
        "config.parse.canonicalize" => "config path",
        "config.parse.path_resolver" => "config resolver",
        "config.parse.source_set" => "config sources",
        "config.parse.prepare_files_total" => "config files",
        "config.parse.headers" => "config headers",
        "config.parse.dependency_sort" => "config sort",
        "config.parse.ast" => "config ast",
        "stage0.single_file.total" => "stage0 single",
        "stage0.single_file.entry_canonicalize" => "entry path",
        "stage0.single_file.path_resolver" => "path resolver",
        "stage0.single_file.reachable_files" => "reachable files",
        "stage0.single_file.string_table_fork" => "string table fork",
        "stage0.single_file.compile_module" => "compile module",
        "stage0.single_file.merge_delta" => "merge delta",
        "stage0.directory.total" => "stage0 dir",
        "stage0.directory.path_resolver" => "path resolver",
        "stage0.directory.module_inventory" => "module inventory",
        "stage0.directory.module_compile_batch" => "module compile",
        "stage0.directory.result_sort" => "result sort",
        "stage0.directory.failure_aggregation" => "failure aggregation",
        "stage0.directory.success_merge" => "success merge",
        "stage0.module_root_discovery.total" => "module roots",
        "stage0.reachable_discovery.total" => "reachable discovery",
        "stage0.reachable_discovery.import_scan" => "import scan",
        "stage0.reachable_discovery.import_resolve" => "import resolve",
        "stage0.reachable_discovery.provider_imports" => "provider imports",
        "stage0.reachable_discovery.source_load" => "source load",
        // Dotted frontend-stage metrics (timers feature).
        "frontend.module.total" => "frontend module",
        "frontend.file_prepare" => "file prep",
        "frontend.dependency_sort" => "sort",
        "frontend.ast" => "ast",
        "frontend.hir" => "hir",
        "frontend.borrow" => "borrow",
        // Dotted backend metrics.
        "backend.html.total" => "html backend",
        "backend.html.site_config" => "site config",
        "backend.html.document_config" => "document config",
        "backend.html.entry_path_plan" => "entry plan",
        "backend.html.module_compile_total" => "html modules",
        "backend.html.external_runtime_assets" => "runtime assets",
        "backend.html.external_runtime_glue" => "runtime glue",
        "backend.html.tracked_assets_plan" => "asset plan",
        "backend.html.tracked_assets_emit" => "asset emit",
        "backend.js.lower_hir" => "js lower",
        "backend.js.generate_module_glue" => "js glue",
        "backend.js.render_html_document" => "html render",
        "backend.wasm.total" => "wasm backend",
        "backend.wasm.lower_wasm" => "wasm lower",
        "backend.wasm.bootstrap_js" => "wasm bootstrap",
        "backend.wasm.artifact_assembly" => "wasm artifacts",
        // Legacy detailed_timers metric names.
        "tokenize_ms" => "tokenize",
        "headers_ms" => "headers",
        "file_prepare_ms" => "file prep",
        "dependency_sort_ms" => "sort",
        "ast_ms" => "ast",
        "ast_build_environment_ms" => "ast env",
        "ast_emit_nodes_ms" => "ast emit",
        "ast_finalize_ms" => "ast finalize",
        "ast_function_body_parse_ms" => "ast func bodies",
        "ast_start_body_parse_ms" => "ast start body",
        "ast_const_template_parse_ms" => "ast const parse",
        "ast_const_template_fold_ms" => "ast const fold",
        "hir_ms" => "hir",
        "borrow_ms" => "borrow",
        _ => stage_name,
    }
}

/// Format a stage movement line for terminal or summary output.
///
/// Returns `None` when there are no meaningful stage movers.
pub fn format_stage_movement_line(
    movements: &[BenchmarkStageMovement],
    thresholds: &BenchmarkThresholds,
) -> Option<String> {
    let meaningful: Vec<&BenchmarkStageMovement> = movements
        .iter()
        .filter(|movement| movement.total_delta_ms.abs() >= thresholds.minimum_stage_delta_ms)
        .take(3)
        .collect();

    if meaningful.is_empty() {
        return None;
    }

    let parts: Vec<String> = meaningful
        .iter()
        .map(|movement| {
            format!(
                "{} {}",
                friendly_stage_label(&movement.stage_name),
                format_signed_ms(movement.total_delta_ms)
            )
        })
        .collect();

    Some(format!("Stage movement: {}", parts.join(", ")))
}

/// Format the top current stages by absolute time for baseline runs.
///
/// Returns `None` when no stage data exists in the current cases.
pub fn format_top_current_stages(cases: &[BenchmarkCaseResult]) -> Option<String> {
    let mut sums_by_name: std::collections::BTreeMap<String, (f64, usize)> =
        std::collections::BTreeMap::new();

    for case in cases {
        for metric in &case.observations.stage_timings {
            let entry = sums_by_name.entry(metric.name.clone()).or_insert((0.0, 0));
            entry.0 += metric.value;
            entry.1 += 1;
        }
    }

    if sums_by_name.is_empty() {
        return None;
    }

    let mut stages: Vec<(String, f64)> = sums_by_name
        .into_iter()
        .map(|(name, (sum, count))| (name, if count == 0 { 0.0 } else { sum / count as f64 }))
        .collect();

    stages.sort_by(|left, right| right.1.total_cmp(&left.1));

    let parts: Vec<String> = stages
        .iter()
        .take(3)
        .map(|(name, value)| format!("{} ~{}ms", friendly_stage_label(name), value.round() as i64))
        .collect();

    Some(format!("Top stages: {}", parts.join(", ")))
}

#[cfg(test)]
mod tests;
