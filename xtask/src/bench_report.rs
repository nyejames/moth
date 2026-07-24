//! Local-only benchmark drilldown report.
//!
//! WHAT: Reads ignored JSONL benchmark history and prints a compact report for
//! optimization investigations.
//! WHY: Tracked summaries stay terse; this command gives developers local
//! per-case, stage, counter, and ratio evidence without writing any files.

use crate::bench_history::{
    LocalRunRecord, RUNS_JSONL_PATH, read_local_runs, thread_identity_label, to_case_results,
};
use crate::bench_system::{SystemIdentityMode, load_or_create_system};
use crate::bench_types::{
    BenchmarkCaseResult, BenchmarkComparison, BenchmarkMetric, BenchmarkStageMovement,
    BenchmarkSuiteKind, BenchmarkSystem, BenchmarkThresholds, calculate_stage_movement,
};
use crate::profile::drift::{
    DriftCaseInput, DriftHotFunction, compute_drift, find_comparable_previous,
};
use crate::profile::history::{PROFILE_RUNS_JSONL_PATH, ProfileHistoryRecord, read_profile_runs};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SLOW_CASE_LIMIT: usize = 3;
const SLOW_CASE_STAGE_LIMIT: usize = 2;
const STAGE_MOVEMENT_LIMIT: usize = 5;
const COUNTER_MOVEMENT_LIMIT: usize = 3;
const RATIO_LIMIT: usize = 5;
const INVESTIGATION_LIMIT: usize = 3;
const MEANINGFUL_COUNTER_RATIO: f64 = 0.03;
const UNATTRIBUTED_LIMIT: usize = 3;
const UNATTRIBUTED_MS_THRESHOLD: f64 = 100.0;
const UNATTRIBUTED_RATIO_THRESHOLD: f64 = 0.25;

const RATIO_CATALOG: &[RatioSpec] = &[
    RatioSpec::new(
        "frontend.file_prepare/source_file_count",
        "frontend.file_prepare",
        "source_file_count",
        "ms/file",
        Some("inspect tokenization, header parsing, string-table merge/remap"),
    ),
    RatioSpec::new(
        "frontend.file_prepare/source_byte_count",
        "frontend.file_prepare",
        "source_byte_count",
        "ms/byte",
        None,
    ),
    RatioSpec::new(
        "frontend.file_prepare/token_count",
        "frontend.file_prepare",
        "token_count",
        "ms/token",
        None,
    ),
    RatioSpec::new(
        "frontend.file_prepare/string_table_delta_entries_scanned",
        "frontend.file_prepare",
        "string_table_delta_entries_scanned",
        "ms/delta-entry",
        Some("inspect string-table delta merge/remap pressure"),
    ),
    RatioSpec::new(
        "frontend.file_prepare/file_prepare_output_remap_calls",
        "frontend.file_prepare",
        "file_prepare_output_remap_calls",
        "ms/output-remap",
        Some("inspect unconditional per-file payload remapping"),
    ),
    RatioSpec::new(
        "frontend.dependency_sort/dependency_edge_count",
        "frontend.dependency_sort",
        "dependency_edge_count",
        "ms/edge",
        Some("inspect duplicate edges or graph traversal"),
    ),
    RatioSpec::new(
        "frontend.ast/ast_header_count",
        "frontend.ast",
        "ast_header_count",
        "ms/header",
        None,
    ),
    RatioSpec::new(
        "ast_build_environment_ms/ast_type_resolution_calls",
        "ast_build_environment_ms",
        "ast_type_resolution_calls",
        "ms/type-resolution",
        Some("inspect repeated AST type resolution"),
    ),
    RatioSpec::new(
        "ast_build_environment_ms/ast_visible_type_lookup_attempts",
        "ast_build_environment_ms",
        "ast_visible_type_lookup_attempts",
        "ms/type-lookup",
        Some("inspect visible type/source lookup paths"),
    ),
    RatioSpec::new(
        "ast_emit_nodes_ms/ast_template_render_plans_built",
        "ast_emit_nodes_ms",
        "ast_template_render_plans_built",
        "ms/render-plan",
        Some("inspect template render-plan build pressure"),
    ),
    RatioSpec::new(
        "ast_emit_nodes_ms/ast_template_render_pieces_built",
        "ast_emit_nodes_ms",
        "ast_template_render_pieces_built",
        "ms/render-piece",
        None,
    ),
    RatioSpec::new(
        "ast_finalize_ms/ast_templates_folded_during_finalization",
        "ast_finalize_ms",
        "ast_templates_folded_during_finalization",
        "ms/finalized-template",
        Some("inspect template finalization and folding"),
    ),
    RatioSpec::new(
        "ast_finalize_ms/ast_template_fold_plan_pieces_visited",
        "ast_finalize_ms",
        "ast_template_fold_plan_pieces_visited",
        "ms/fold-piece",
        None,
    ),
    RatioSpec::new(
        "frontend.ast/type_compatibility_cache_lookups",
        "frontend.ast",
        "type_compatibility_cache_lookups",
        "ms/lookup",
        None,
    ),
    RatioSpec::new(
        "frontend.ast/type_compatibility_cache_misses",
        "frontend.ast",
        "type_compatibility_cache_misses",
        "ms/miss",
        Some("inspect compatibility caching or repeated type checks"),
    ),
    RatioSpec::new(
        "frontend.hir/hir_statement_count",
        "frontend.hir",
        "hir_statement_count",
        "ms/statement",
        None,
    ),
    RatioSpec::new(
        "frontend.borrow/borrow_conflict_check_count",
        "frontend.borrow",
        "borrow_conflict_check_count",
        "ms/check",
        Some("inspect borrow state representation"),
    ),
    RatioSpec::new(
        "frontend.borrow/borrow_state_join_count",
        "frontend.borrow",
        "borrow_state_join_count",
        "ms/join",
        Some("inspect borrow state join pressure"),
    ),
    RatioSpec::new(
        "frontend.borrow/borrow_place_access_count",
        "frontend.borrow",
        "borrow_place_access_count",
        "ms/place-access",
        None,
    ),
    RatioSpec::new(
        "frontend.borrow/borrow_statement_fact_count",
        "frontend.borrow",
        "borrow_statement_fact_count",
        "ms/statement-fact",
        None,
    ),
    RatioSpec::new(
        "frontend.borrow/borrow_value_fact_count",
        "frontend.borrow",
        "borrow_value_fact_count",
        "ms/value-fact",
        None,
    ),
];

/// Run `bench-report` from the repository root.
pub fn run_benchmark_report() -> Result<(), String> {
    let runs = read_local_runs(Path::new(RUNS_JSONL_PATH))?;
    let system = load_or_create_system(SystemIdentityMode::ReadOnly)?;
    let mut report = calculate_benchmark_report(&runs, system.as_ref());

    // Collect the latest profile run info (silently omitted if missing/malformed).
    report.latest_profile_run = collect_latest_profile_run(system.as_ref());

    println!("{}", format_benchmark_report(&report));

    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkReport {
    pub(crate) used_current_system: bool,
    pub(crate) suites: Vec<SuiteReport>,
    pub(crate) latest_profile_run: Option<LatestProfileRun>,
}

/// Compact data for the "Latest profile run" section of the bench report.
///
/// WHAT: Shows the most recent profiling run's identity, filter mode, case
/// count, and a single top drift item if a comparable previous run exists.
///
/// WHY: `bench-report` is the first stop for optimization work. Surfacing
/// the latest profile run here lets developers jump to the profiling
/// artifacts without leaving the report. The section stays compact to
/// avoid duplicating the full `profile-drift.md` table.
#[derive(Debug, Clone)]
pub(crate) struct LatestProfileRun {
    /// Run identifier (e.g., "2026-06-18T10-30-abc1234").
    pub(crate) run_id: String,
    /// Filter mode label ("terse", "normal", "deep", "raw-index").
    pub(crate) filter_mode: String,
    /// Number of cases profiled in this run.
    pub(crate) case_count: usize,
    /// Concise description of the top drift item, or "none" if no
    /// comparable previous record exists or no drift exceeded thresholds.
    pub(crate) top_drift_item: String,
    /// Relative path to the run's `agent-summary.md`.
    pub(crate) agent_summary_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SuiteReport {
    pub(crate) suite_kind: BenchmarkSuiteKind,
    pub(crate) system_display: String,
    pub(crate) public_system_id: String,
    pub(crate) latest_timestamp: String,
    pub(crate) latest_commit: Option<String>,
    pub(crate) thread_count: Option<u32>,
    pub(crate) comparison: BenchmarkComparison,
    pub(crate) slowest_cases: Vec<SlowCaseReport>,
    pub(crate) unattributed_cases: Vec<UnattributedCaseReport>,
    pub(crate) stage_movements: Vec<BenchmarkStageMovement>,
    pub(crate) counter_movements: Vec<CounterMovement>,
    pub(crate) ratios: Vec<RatioReport>,
    pub(crate) investigation_hints: Vec<InvestigationHint>,
}

#[derive(Debug, Clone)]
pub(crate) struct SlowCaseReport {
    pub(crate) name: String,
    pub(crate) mean_ms: f64,
    pub(crate) stages: Vec<BenchmarkMetric>,
}

#[derive(Debug, Clone)]
pub(crate) struct UnattributedCaseReport {
    pub(crate) name: String,
    pub(crate) unattributed_ms: f64,
    pub(crate) unattributed_ratio: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct CounterMovement {
    pub(crate) name: String,
    pub(crate) previous: f64,
    pub(crate) delta: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct RatioReport {
    pub(crate) name: &'static str,
    pub(crate) case_name: String,
    pub(crate) value: f64,
    pub(crate) unit: &'static str,
    pub(crate) hint: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct InvestigationHint {
    pub(crate) case_name: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Copy)]
struct RatioSpec {
    name: &'static str,
    numerator: &'static str,
    denominator: &'static str,
    unit: &'static str,
    hint: Option<&'static str>,
}

impl RatioSpec {
    const fn new(
        name: &'static str,
        numerator: &'static str,
        denominator: &'static str,
        unit: &'static str,
        hint: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            numerator,
            denominator,
            unit,
            hint,
        }
    }
}

pub(crate) fn calculate_benchmark_report(
    runs: &[LocalRunRecord],
    current_system: Option<&BenchmarkSystem>,
) -> BenchmarkReport {
    let mut suites = Vec::new();

    for suite_kind in [
        BenchmarkSuiteKind::EndToEndCli,
        BenchmarkSuiteKind::FrontendPhases,
    ] {
        let Some(selection) = select_latest_run(runs, suite_kind, current_system) else {
            continue;
        };

        let suite = calculate_suite_report(suite_kind, selection);
        suites.push(suite);
    }

    BenchmarkReport {
        used_current_system: current_system.is_some(),
        suites,
        latest_profile_run: None,
    }
}

fn select_latest_run<'a>(
    runs: &'a [LocalRunRecord],
    suite_kind: BenchmarkSuiteKind,
    current_system: Option<&BenchmarkSystem>,
) -> Option<SelectedRun<'a>> {
    let persisted_suite_kind = suite_kind.persisted_name();

    let latest_index = runs.iter().enumerate().rev().find_map(|(index, run)| {
        if run.suite_kind != persisted_suite_kind {
            return None;
        }

        if let Some(system) = current_system
            && run.system_uuid != system.system_uuid
        {
            return None;
        }

        Some(index)
    })?;
    let latest = &runs[latest_index];

    // The previous run must match the latest run's thread identity so the
    // report never compares runs with different parallelism levels.
    let previous = runs[..latest_index].iter().rev().find(|run| {
        run.suite_kind == persisted_suite_kind
            && run.system_uuid == latest.system_uuid
            && run.thread_count == latest.thread_count
    });

    Some(SelectedRun { latest, previous })
}

struct SelectedRun<'a> {
    latest: &'a LocalRunRecord,
    previous: Option<&'a LocalRunRecord>,
}

fn calculate_suite_report(
    suite_kind: BenchmarkSuiteKind,
    selection: SelectedRun<'_>,
) -> SuiteReport {
    let current_cases = to_case_results(selection.latest);
    let previous_cases = selection.previous.map(to_case_results);
    let comparison = BenchmarkComparison::new(&current_cases, previous_cases.as_deref());

    let slowest_cases = calculate_slowest_cases(&current_cases);
    let unattributed_cases = calculate_unattributed_cases(suite_kind, &current_cases);
    let stage_movements = calculate_meaningful_stage_movements(&comparison);
    let counter_movements = calculate_counter_movements(&current_cases, previous_cases.as_deref());
    let ratios = calculate_ratios(&current_cases);
    let investigation_hints = calculate_investigation_hints(
        &comparison,
        &current_cases,
        &counter_movements,
        &ratios,
        previous_cases.as_deref(),
    );

    SuiteReport {
        suite_kind,
        system_display: selection.latest.display_name.clone(),
        public_system_id: selection.latest.public_system_id.clone(),
        latest_timestamp: selection.latest.timestamp.clone(),
        latest_commit: selection.latest.commit.clone(),
        thread_count: selection.latest.thread_count,
        comparison,
        slowest_cases,
        unattributed_cases,
        stage_movements,
        counter_movements,
        ratios,
        investigation_hints,
    }
}

fn calculate_slowest_cases(cases: &[BenchmarkCaseResult]) -> Vec<SlowCaseReport> {
    let mut sorted_cases: Vec<&BenchmarkCaseResult> = cases.iter().collect();
    sorted_cases.sort_by(|left, right| right.mean_ms.total_cmp(&left.mean_ms));

    sorted_cases
        .into_iter()
        .take(SLOW_CASE_LIMIT)
        .map(|case| {
            let mut stages = case.observations.stage_timings.clone();
            stages.sort_by(|left, right| right.value.total_cmp(&left.value));
            stages.truncate(SLOW_CASE_STAGE_LIMIT);

            SlowCaseReport {
                name: case.case_name.clone(),
                mean_ms: case.mean_ms,
                stages,
            }
        })
        .collect()
}

fn calculate_unattributed_cases(
    suite_kind: BenchmarkSuiteKind,
    cases: &[BenchmarkCaseResult],
) -> Vec<UnattributedCaseReport> {
    if suite_kind != BenchmarkSuiteKind::EndToEndCli {
        return Vec::new();
    }

    let mut reports = Vec::new();
    for case in cases {
        let Some(attributed_ms) = attributed_wall_time_ms(case) else {
            continue;
        };

        let unattributed_ms = (case.mean_ms - attributed_ms).max(0.0);
        let unattributed_ratio = if case.mean_ms > 0.0 {
            unattributed_ms / case.mean_ms
        } else {
            0.0
        };

        if unattributed_ms < UNATTRIBUTED_MS_THRESHOLD
            && unattributed_ratio < UNATTRIBUTED_RATIO_THRESHOLD
        {
            continue;
        }

        reports.push(UnattributedCaseReport {
            name: case.case_name.clone(),
            unattributed_ms,
            unattributed_ratio,
        });
    }

    reports.sort_by(|left, right| right.unattributed_ms.total_cmp(&left.unattributed_ms));
    reports.truncate(UNATTRIBUTED_LIMIT);
    reports
}

fn attributed_wall_time_ms(case: &BenchmarkCaseResult) -> Option<f64> {
    // Top-level non-nested pipeline phases for each command. These are the
    // dotted metrics emitted by the concise `timers` feature under
    // MOTH_TIMERS=bench. We deliberately exclude `.total` and `_total`
    // wrapper metrics (e.g. command.check.total, build_project.total,
    // output.write_total) because they nest the sub-phases listed here and
    // would double-count if summed alongside them.
    let command_phase_names = match case.command.as_str() {
        "check" => &[
            "command.check.path_validation",
            "command.check.builder_construction",
            "command.check.bootstrap",
            "command.check.compile_project_frontend",
            "command.check.message_rendering",
        ][..],
        "build" => &[
            "build_project.path_validation",
            "build_project.bootstrap",
            "build_project.compile_project_frontend",
            "build_project.backend",
            "command.build.output_write",
        ][..],
        _ => &[][..],
    };

    let command_phase_sum = command_phase_names
        .iter()
        .filter_map(|name| metric_value(&case.observations.stage_timings, name))
        .sum::<f64>();
    if command_phase_sum > 0.0 {
        return Some(command_phase_sum);
    }

    // Legacy benchmark records only carried lower-level frontend stage timers.
    // Keeping this fallback makes old attribution holes visible in bench-report
    // instead of hiding them until all command-phase metrics are present.
    let fallback_sum = case
        .observations
        .stage_timings
        .iter()
        .filter(|metric| !is_total_wrapper_metric(&metric.name))
        .map(|metric| metric.value)
        .sum::<f64>();
    if fallback_sum > 0.0 {
        Some(fallback_sum)
    } else {
        None
    }
}

/// Whether a metric name is a total/wrapper that nests sub-phases.
///
/// WHAT: matches dotted totals (e.g. `command.check.total`,
/// `build_project.total`, `stage0.single_file.total`) and underscore
/// totals (e.g. `output.write_total`, `config.load_total`) so the
/// fallback attribution sum does not double-count nested phases.
/// WHY: both naming conventions exist in the current metric set;
/// a single predicate keeps the filter consistent as new stages land.
fn is_total_wrapper_metric(name: &str) -> bool {
    name.ends_with(".total") || name.ends_with("_total")
}

fn calculate_meaningful_stage_movements(
    comparison: &BenchmarkComparison,
) -> Vec<BenchmarkStageMovement> {
    calculate_stage_movement(comparison)
        .into_iter()
        .filter(|movement| {
            movement.total_delta_ms.abs() >= BenchmarkThresholds::DEFAULT.minimum_stage_delta_ms
        })
        .take(STAGE_MOVEMENT_LIMIT)
        .collect()
}

fn calculate_counter_movements(
    current_cases: &[BenchmarkCaseResult],
    previous_cases: Option<&[BenchmarkCaseResult]>,
) -> Vec<CounterMovement> {
    let Some(previous_cases) = previous_cases else {
        return Vec::new();
    };

    let current_totals = sum_counters(current_cases);
    let previous_totals = sum_counters(previous_cases);
    let mut names = BTreeSet::new();
    names.extend(current_totals.keys().cloned());
    names.extend(previous_totals.keys().cloned());

    let mut movements = Vec::new();
    for name in names {
        let current = current_totals.get(&name).copied().unwrap_or(0.0);
        let previous = previous_totals.get(&name).copied().unwrap_or(0.0);
        let delta = current - previous;

        if !is_meaningful_counter_movement(previous, delta) {
            continue;
        }

        movements.push(CounterMovement {
            name,
            previous,
            delta,
        });
    }

    movements.sort_by(|left, right| counter_score(right).total_cmp(&counter_score(left)));
    movements.truncate(COUNTER_MOVEMENT_LIMIT);
    movements
}

fn sum_counters(cases: &[BenchmarkCaseResult]) -> BTreeMap<String, f64> {
    let mut totals = BTreeMap::new();

    for case in cases {
        for counter in &case.observations.counters {
            let entry = totals.entry(counter.name.clone()).or_insert(0.0);
            *entry += counter.value;
        }
    }

    totals
}

fn is_meaningful_counter_movement(previous: f64, delta: f64) -> bool {
    if previous == 0.0 {
        return delta != 0.0;
    }

    (delta / previous).abs() >= MEANINGFUL_COUNTER_RATIO
}

fn counter_score(movement: &CounterMovement) -> f64 {
    if movement.previous == 0.0 {
        movement.delta.abs()
    } else {
        (movement.delta / movement.previous).abs()
    }
}

fn calculate_ratios(cases: &[BenchmarkCaseResult]) -> Vec<RatioReport> {
    let mut ratios = Vec::new();

    for case in cases {
        for spec in RATIO_CATALOG {
            let Some(numerator) = metric_value(&case.observations.stage_timings, spec.numerator)
            else {
                continue;
            };
            let Some(denominator) = metric_value(&case.observations.counters, spec.denominator)
            else {
                continue;
            };

            if denominator == 0.0 {
                continue;
            }

            ratios.push(RatioReport {
                name: spec.name,
                case_name: case.case_name.clone(),
                value: numerator / denominator,
                unit: spec.unit,
                hint: spec.hint,
            });
        }
    }

    ratios.sort_by(|left, right| right.value.total_cmp(&left.value));
    ratios.truncate(RATIO_LIMIT);
    ratios
}

fn metric_value(metrics: &[BenchmarkMetric], name: &str) -> Option<f64> {
    let exact = metrics
        .iter()
        .find(|metric| metric.name == name)
        .map(|metric| metric.value);
    if exact.is_some() {
        return exact;
    }

    legacy_stage_metric_alias(name).and_then(|legacy_name| {
        metrics
            .iter()
            .find(|metric| metric.name == legacy_name)
            .map(|metric| metric.value)
    })
}

fn legacy_stage_metric_alias(name: &str) -> Option<&'static str> {
    match name {
        "frontend.file_prepare" => Some("file_prepare_ms"),
        "frontend.dependency_sort" => Some("dependency_sort_ms"),
        "frontend.ast" => Some("ast_ms"),
        "frontend.hir" => Some("hir_ms"),
        "frontend.borrow" => Some("borrow_ms"),
        _ => None,
    }
}

fn calculate_investigation_hints(
    comparison: &BenchmarkComparison,
    current_cases: &[BenchmarkCaseResult],
    counter_movements: &[CounterMovement],
    ratios: &[RatioReport],
    previous_cases: Option<&[BenchmarkCaseResult]>,
) -> Vec<InvestigationHint> {
    let mut hints = Vec::new();
    let mut seen = BTreeSet::new();

    for ratio in ratios {
        let Some(message) = ratio.hint else {
            continue;
        };

        push_hint(
            &mut hints,
            &mut seen,
            ratio.case_name.clone(),
            format!("high {}; {}", ratio.name, message),
        );

        if hints.len() >= INVESTIGATION_LIMIT {
            return hints;
        }
    }

    if let Some(previous_cases) = previous_cases {
        for case in &comparison.cases {
            if case.delta_ms <= case.threshold_ms {
                continue;
            }

            if case_counters_are_stable(case, current_cases, previous_cases, counter_movements) {
                push_hint(
                    &mut hints,
                    &mut seen,
                    case.case_name.clone(),
                    "timing grew while counters stayed near previous values; run CPU profiling before refactoring"
                        .to_string(),
                );
            }

            if hints.len() >= INVESTIGATION_LIMIT {
                break;
            }
        }
    }

    hints
}

fn push_hint(
    hints: &mut Vec<InvestigationHint>,
    seen: &mut BTreeSet<String>,
    case_name: String,
    message: String,
) {
    let key = format!("{case_name}\n{message}");
    if seen.insert(key) {
        hints.push(InvestigationHint { case_name, message });
    }
}

fn case_counters_are_stable(
    case: &crate::bench_types::BenchmarkCaseComparison,
    current_cases: &[BenchmarkCaseResult],
    previous_cases: &[BenchmarkCaseResult],
    counter_movements: &[CounterMovement],
) -> bool {
    let Some(previous_case) = previous_cases
        .iter()
        .find(|previous| previous.case_name == case.case_name)
    else {
        return false;
    };
    let Some(current_case) = current_cases
        .iter()
        .find(|current| current.case_name == case.case_name)
    else {
        return false;
    };

    let previous_total = previous_case
        .observations
        .counters
        .iter()
        .map(|counter| counter.value)
        .sum::<f64>();
    let current_total = current_case
        .observations
        .counters
        .iter()
        .map(|counter| counter.value)
        .sum::<f64>();

    if previous_total == 0.0 && current_total == 0.0 {
        return false;
    }

    if counter_movements.is_empty() {
        return true;
    }

    if previous_total == 0.0 {
        return current_total == 0.0;
    }

    ((current_total - previous_total) / previous_total).abs() < MEANINGFUL_COUNTER_RATIO
}

pub(crate) fn format_benchmark_report(report: &BenchmarkReport) -> String {
    let mut output = String::from("Benchmark report: local data only\n");

    if !report.used_current_system {
        output.push_str("\nNo local system identity found; showing latest local run per suite.\n");
    }

    if report.suites.is_empty() {
        if report.used_current_system {
            output.push_str("\nNo local benchmark history found for the current system.\n");
        } else {
            output.push_str("\nNo local benchmark history found.\n");
        }
        // Still show the latest profile run even when benchmark history is empty.
        append_latest_profile_run(&mut output, report);
        return output;
    }

    for suite in &report.suites {
        output.push('\n');
        output.push_str(&format!(
            "{} / {} ({})\n",
            suite.suite_kind.display_label(),
            suite.system_display,
            suite.public_system_id
        ));
        output.push_str(&format!(
            "Latest: {}, commit {}\n",
            suite.latest_timestamp,
            suite.latest_commit.as_deref().unwrap_or("unknown")
        ));
        output.push_str(&format!(
            "Threads: {}\n",
            thread_identity_label(suite.thread_count)
        ));
        output.push_str(&format!(
            "Change: {}\n",
            suite.comparison.format_run_change_line().replace("**", "")
        ));

        append_slowest_cases(&mut output, suite);
        append_unattributed_cases(&mut output, suite);
        append_stage_movement(&mut output, suite);
        append_counter_movement(&mut output, suite);
        append_ratios(&mut output, suite);
        append_investigation_hints(&mut output, suite);
    }

    append_latest_profile_run(&mut output, report);

    output
}

fn append_slowest_cases(output: &mut String, suite: &SuiteReport) {
    output.push_str("\nSlowest cases:\n");
    if suite.slowest_cases.is_empty() {
        output.push_str("  none\n");
        return;
    }

    for case in &suite.slowest_cases {
        let stage_text = case
            .stages
            .iter()
            .map(|stage| format!("{} ~{}ms", stage.name, stage.value.round() as i64))
            .collect::<Vec<_>>()
            .join(", ");

        if stage_text.is_empty() {
            output.push_str(&format!(
                "  {:<28} ~{}ms\n",
                case.name,
                case.mean_ms.round() as i64
            ));
        } else {
            output.push_str(&format!(
                "  {:<28} ~{}ms  {}\n",
                case.name,
                case.mean_ms.round() as i64,
                stage_text
            ));
        }
    }
}

fn append_unattributed_cases(output: &mut String, suite: &SuiteReport) {
    if suite.suite_kind != BenchmarkSuiteKind::EndToEndCli {
        return;
    }

    output.push_str("\nUnattributed wall time:\n");
    if suite.unattributed_cases.is_empty() {
        output.push_str("  none\n");
        return;
    }

    for case in &suite.unattributed_cases {
        output.push_str(&format!(
            "  {:<28} ~{}ms ({:.0}%)\n",
            case.name,
            case.unattributed_ms.round() as i64,
            case.unattributed_ratio * 100.0
        ));
    }
}

fn append_stage_movement(output: &mut String, suite: &SuiteReport) {
    output.push_str("\nStage movement:\n");
    if suite.stage_movements.is_empty() {
        output.push_str("  none\n");
        return;
    }

    for movement in &suite.stage_movements {
        output.push_str(&format!(
            "  {:<32} {} across {} cases\n",
            movement.stage_name,
            format_signed_ms(movement.total_delta_ms),
            movement.case_count
        ));
    }
}

fn append_counter_movement(output: &mut String, suite: &SuiteReport) {
    output.push_str("\nCounter movement:\n");
    if suite.counter_movements.is_empty() {
        output.push_str("  none\n");
        return;
    }

    for movement in &suite.counter_movements {
        output.push_str(&format!(
            "  {:<32} {}\n",
            movement.name,
            format_counter_delta(movement)
        ));
    }
}

fn append_ratios(output: &mut String, suite: &SuiteReport) {
    output.push_str("\nRatios:\n");
    if suite.ratios.is_empty() {
        output.push_str("  none\n");
        return;
    }

    for ratio in &suite.ratios {
        output.push_str(&format!(
            "  {:<40} {:<24} {}{}\n",
            ratio.name,
            ratio.case_name,
            format_ratio_value(ratio.value),
            ratio.unit
        ));
    }
}

fn append_investigation_hints(output: &mut String, suite: &SuiteReport) {
    output.push_str("\nNext investigation candidates:\n");
    if suite.investigation_hints.is_empty() {
        output.push_str("  none\n");
        return;
    }

    for hint in &suite.investigation_hints {
        output.push_str(&format!("  {}: {}\n", hint.case_name, hint.message));
    }
}

fn format_counter_delta(movement: &CounterMovement) -> String {
    if movement.previous == 0.0 {
        return format_signed_number(movement.delta);
    }

    let percent = (movement.delta / movement.previous) * 100.0;
    format!("{}%", format_signed_number(percent))
}

fn format_signed_ms(value: f64) -> String {
    format!("{}ms", format_signed_number(value.round()))
}

fn format_signed_number(value: f64) -> String {
    let rounded = value.round() as i64;
    if rounded > 0 {
        format!("+{rounded}")
    } else {
        rounded.to_string()
    }
}

fn format_ratio_value(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.1}")
    } else if value >= 1.0 {
        format!("{value:.2}")
    } else {
        format!("{value:.4}")
    }
}

// ---------------------------------------------------------------------------
//  Latest profile run integration
// ---------------------------------------------------------------------------

/// Append the "Latest profile run" section to the report output.
///
/// WHAT: Renders a compact section showing the latest profiling run's
/// identity, filter mode, case count, top drift item, and path to the
/// agent summary.
///
/// WHY: `bench-report` is the first stop for optimization work. Pointing
/// to the latest profile run lets developers jump to profiling artifacts
/// without leaving the report. The section stays compact to avoid
/// duplicating the full `profile-drift.md` table.
fn append_latest_profile_run(output: &mut String, report: &BenchmarkReport) {
    let Some(profile_run) = &report.latest_profile_run else {
        return;
    };

    output.push_str("\nLatest profile run:\n");
    output.push_str(&format!("  Run:       {}\n", profile_run.run_id));
    output.push_str(&format!("  Filter:    {}\n", profile_run.filter_mode));
    output.push_str(&format!("  Cases:     {}\n", profile_run.case_count));
    output.push_str(&format!("  Top drift: {}\n", profile_run.top_drift_item));
    output.push_str(&format!(
        "  Summary:   {}\n",
        profile_run.agent_summary_path
    ));
}

/// Collect the latest profile run info from `profile-runs.jsonl`.
///
/// WHAT: Reads the profile history file, finds the latest record, compares
/// it against the most recent previous comparable record to determine the
/// top drift item, and returns a compact `LatestProfileRun`.
///
/// WHY: Returns `None` silently when the file is missing, empty, or
/// malformed so `bench-report` never depends on profile data existing.
fn collect_latest_profile_run(
    current_system: Option<&BenchmarkSystem>,
) -> Option<LatestProfileRun> {
    let history_path = Path::new(PROFILE_RUNS_JSONL_PATH);
    let records = read_profile_runs(history_path).ok()?;

    let latest = records.last()?;

    // Filter by system UUID if a system identity is available.
    if let Some(system) = current_system
        && latest.system_uuid != system.system_uuid
    {
        return None;
    }

    let case_count = latest.cases.len();
    let agent_summary_path = latest
        .cases
        .first()
        .map(|c| format!("{}/agent-summary.md", c.run_directory_path))
        .unwrap_or_else(|| {
            format!(
                "benchmarks/local-data/profiles/{}/agent-summary.md",
                latest.run_id
            )
        });

    // Find the comparable previous record for drift.
    let top_drift_item = format_top_drift_item(&records, current_system, latest);

    Some(LatestProfileRun {
        run_id: latest.run_id.clone(),
        filter_mode: latest.filter_mode.clone(),
        case_count,
        top_drift_item,
        agent_summary_path,
    })
}

/// Determine the top drift item by comparing the latest record against
/// the most recent previous comparable record.
///
/// WHAT: Uses `find_comparable_previous` and `compute_drift` from the
/// profile drift module to find the single most significant function
/// drift. Returns a concise one-line description or "none".
///
/// WHY: A single top drift item gives a quick pointer to the most
/// interesting change without duplicating the full drift table.
fn format_top_drift_item(
    records: &[ProfileHistoryRecord],
    current_system: Option<&BenchmarkSystem>,
    latest: &ProfileHistoryRecord,
) -> String {
    let system_uuid = current_system
        .map(|s| s.system_uuid.as_str())
        .unwrap_or("unknown");

    let previous = find_comparable_previous(
        records,
        system_uuid,
        &latest.filter_mode,
        latest.sample_rate_hz,
        &latest.run_id,
    );

    let Some(prev) = previous else {
        return "none".to_string();
    };

    // Build drift case inputs from the latest record's case data.
    let drift_cases: Vec<DriftCaseInput> = latest
        .cases
        .iter()
        .map(|case| {
            let hot_functions: Vec<DriftHotFunction> = case
                .hot_functions
                .iter()
                .map(|f| DriftHotFunction {
                    name: f.name.clone(),
                    bucket_label: f.bucket_label.clone(),
                    inclusive_samples: f.inclusive_samples,
                    inclusive_pct: f.inclusive_pct,
                })
                .collect();

            DriftCaseInput {
                case_name: case.case_name.clone(),
                command: case.command.clone(),
                args: case.args.clone(),
                stage_timings: case.stage_timings.clone(),
                counters: case.counters.clone(),
                hot_functions,
            }
        })
        .collect();

    let wall_times: std::collections::HashMap<String, f64> = latest
        .cases
        .iter()
        .map(|case| (case.case_name.clone(), case.observation_wall_ms))
        .collect();

    let drift_report = compute_drift(&drift_cases, prev, &wall_times);

    // Find the single most significant function drift (increase or decrease).
    // Increases are positive deltas; decreases are stored as negative deltas
    // so the final sign is preserved in the output.
    let top_increase = drift_report
        .function_increases
        .first()
        .map(|d| (d.delta_pct, d.function_name.clone(), &d.bucket_label));

    let top_decrease = drift_report
        .function_decreases
        .first()
        .map(|d| (-d.delta_pct.abs(), d.function_name.clone(), &d.bucket_label));

    let top = match (top_increase, top_decrease) {
        (Some((inc_delta, inc_name, inc_bucket)), Some((dec_delta, dec_name, dec_bucket))) => {
            if inc_delta.abs() >= dec_delta.abs() {
                (inc_delta, inc_name, inc_bucket)
            } else {
                (dec_delta, dec_name, dec_bucket)
            }
        }
        (Some((delta, name, bucket)), None) => (delta, name, bucket),
        (None, Some((delta, name, bucket))) => (delta, name, bucket),
        (None, None) => return "none".to_string(),
    };

    // Format: "+9.2pp resolve_type (AST)" or "-9.2pp resolve_type (AST)"
    let short_name = top.1.rsplit("::").next().unwrap_or(&top.1);
    format!("{:+.1}pp {} ({})", top.0, short_name, top.2)
}

#[cfg(test)]
mod tests;
