//! Local raw benchmark history.
//!
//! This module owns the one JSONL persistence path for completed benchmark
//! runs. Current records use serde directly. Explicit legacy adapters keep
//! formats 1 through 5 readable without making their path-derived identities
//! part of the current benchmark domain.

use crate::bench_types::{
    BENCHMARK_PROTOCOL_VERSION, BenchmarkCaseObservations, BenchmarkCaseResult,
    BenchmarkGroupStats, BenchmarkMetric, BenchmarkRun, BenchmarkSuiteKind,
};
use crate::benchmark_manifest::{BenchmarkRunner, CliBenchmarkCommand, FrontendBenchmarkProfile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Path to the local raw benchmark history file, relative to repo root.
pub const RUNS_JSONL_PATH: &str = "benchmarks/local-data/runs.jsonl";

/// Current on-disk format version.
const FORMAT_VERSION: u32 = 7;

/// One benchmark run in the current in-memory history shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRunRecord {
    pub format_version: u32,
    pub benchmark_protocol_version: u32,
    pub timestamp: String,
    pub month_key: String,
    pub commit: Option<String>,
    pub git_dirty: Option<bool>,
    pub system_uuid: String,
    pub public_system_id: String,
    pub display_name: String,
    pub warmup_runs: usize,
    pub measured_iterations: usize,
    pub suite_kind: String,
    pub primary_metric_name: String,
    pub suite_average_ms: f64,
    pub suite_case_spread_ms: f64,
    pub thread_count: Option<u32>,
    pub groups: Vec<LocalGroupRecord>,
    pub cases: Vec<LocalCaseRecord>,
}

/// Aggregated group statistics within a stored run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalGroupRecord {
    pub name: String,
    pub case_count: usize,
    pub average_ms: f64,
}

/// One case result within a stored run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalCaseRecord {
    pub case_id: String,
    pub workload_id: Option<String>,
    pub source_fingerprint: Option<String>,
    pub measurement_fingerprint: Option<String>,
    pub group_name: String,
    pub runner: BenchmarkRunner,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub stddev_ms: f64,
    pub stage_timings: Vec<LocalMetricRecord>,
    pub counters: Vec<LocalMetricRecord>,
}

/// A local-only named detailed timer or counter value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalMetricRecord {
    pub name: String,
    pub value: f64,
}

/// Read all compatible runs from a local JSONL file.
///
/// Malformed lines and unknown future versions remain isolated to their line
/// so one bad local record cannot make the complete history unreadable.
pub fn read_local_runs(path: &Path) -> Result<Vec<LocalRunRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents =
        fs::read_to_string(path).map_err(|error| format!("Failed to read runs.jsonl: {error}"))?;
    let mut runs = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match parse_history_line(trimmed) {
            Ok(Some(record)) => runs.push(record),
            Ok(None) => {}
            Err(error) => {
                eprintln!("Warning: skipping malformed runs.jsonl line: {error}");
            }
        }
    }

    Ok(runs)
}

fn parse_history_line(line: &str) -> Result<Option<LocalRunRecord>, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("invalid JSON: {error}"))?;
    let format_version = value
        .get("format_version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or("missing or invalid format_version")?;

    if format_version > FORMAT_VERSION {
        eprintln!("Warning: skipping future runs.jsonl format version {format_version}");
        return Ok(None);
    }

    let record = match format_version {
        1 => adapt_v1(deserialize_legacy(value, 1)?)?,
        2 => adapt_v2(deserialize_legacy(value, 2)?)?,
        3 => adapt_v3(deserialize_legacy(value, 3)?)?,
        4 => adapt_v4(deserialize_legacy(value, 4)?)?,
        5 => adapt_v5(deserialize_legacy(value, 5)?)?,
        6 => adapt_v6(deserialize_legacy(value, 6)?)?,
        7 => {
            let record: LocalRunRecord = serde_json::from_value(value)
                .map_err(|error| format!("invalid v7 record: {error}"))?;
            validate_v7_record(&record)?;
            record
        }
        _ => return Err(format!("unsupported format_version {format_version}")),
    };

    Ok(Some(record))
}

fn deserialize_legacy<T>(value: Value, version: u32) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(value).map_err(|error| format!("invalid v{version} record: {error}"))
}

fn validate_v7_record(record: &LocalRunRecord) -> Result<(), String> {
    if record.format_version != FORMAT_VERSION {
        return Err(format!(
            "v7 record declared format version {}",
            record.format_version
        ));
    }
    if record.benchmark_protocol_version == 0 {
        return Err("v7 record has legacy protocol version 0".to_string());
    }

    for case in &record.cases {
        if case.case_id.is_empty() {
            return Err("v7 case has empty case_id".to_string());
        }
        if case.workload_id.as_deref().is_none_or(str::is_empty) {
            return Err(format!(
                "v7 case '{}' has missing or empty workload_id",
                case.case_id
            ));
        }
        if case.source_fingerprint.as_deref().is_none_or(str::is_empty) {
            return Err(format!(
                "v7 case '{}' has missing or empty source_fingerprint",
                case.case_id
            ));
        }
        if case
            .measurement_fingerprint
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(format!(
                "v7 case '{}' has missing or empty measurement_fingerprint",
                case.case_id
            ));
        }
    }

    Ok(())
}

/// Find the latest directly comparable run.
pub fn find_latest_matching_run<'a>(
    runs: &'a [LocalRunRecord],
    system_uuid: &str,
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
) -> Option<&'a LocalRunRecord> {
    let persisted_suite_kind = suite_kind.persisted_name();

    runs.iter().rfind(|run| {
        run.system_uuid == system_uuid
            && run.suite_kind == persisted_suite_kind
            && run.thread_count == thread_count
            && run.benchmark_protocol_version == BENCHMARK_PROTOCOL_VERSION
    })
}

/// Append one completed current-format run.
pub fn append_local_run(path: &Path, record: &LocalRunRecord) -> Result<(), String> {
    validate_v7_record(record)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create local-data directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let line = serde_json::to_string(record)
        .map_err(|error| format!("Failed to serialize runs.jsonl record: {error}"))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("Failed to open runs.jsonl: {error}"))?;

    use std::io::Write;
    writeln!(file, "{line}").map_err(|error| format!("Failed to append to runs.jsonl: {error}"))
}

/// Capture the effective `RAYON_NUM_THREADS` setting as a normalized identity.
pub fn effective_thread_count() -> Result<Option<u32>, String> {
    use std::env::VarError;

    match std::env::var("RAYON_NUM_THREADS") {
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => Err(
            "RAYON_NUM_THREADS is set to a non-Unicode value; expected a positive integer or unset for default threads"
                .to_string(),
        ),
        Ok(value) => parse_thread_count(&value),
    }
}

pub fn thread_identity_label(thread_count: Option<u32>) -> String {
    match thread_count {
        None => "default".to_string(),
        Some(count) => format!("fixed: {count}"),
    }
}

pub fn thread_identity_suffix(thread_count: Option<u32>) -> String {
    match thread_count {
        None => String::new(),
        Some(_) => format!(" [threads: {}]", thread_identity_label(thread_count)),
    }
}

fn parse_thread_count(value: &str) -> Result<Option<u32>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(
            "RAYON_NUM_THREADS is set to an empty string; expected a positive integer or unset for default threads"
                .to_string(),
        );
    }
    let count: u32 = trimmed.parse().map_err(|_| {
        format!(
            "RAYON_NUM_THREADS is set to '{value}'; expected a positive integer or unset for default threads"
        )
    })?;
    if count == 0 {
        return Err(
            "RAYON_NUM_THREADS is set to 0; expected a positive integer or unset for default threads"
                .to_string(),
        );
    }

    Ok(Some(count))
}

/// Convert a completed run into the current persisted shape.
pub fn to_local_record(run: &BenchmarkRun) -> LocalRunRecord {
    debug_assert_eq!(
        run.groups
            .iter()
            .map(|group| group.case_count)
            .sum::<usize>(),
        run.cases.len()
    );

    LocalRunRecord {
        format_version: FORMAT_VERSION,
        benchmark_protocol_version: run.benchmark_protocol_version,
        timestamp: format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            run.timestamp.year,
            run.timestamp.month,
            run.timestamp.day,
            run.timestamp.hour,
            run.timestamp.minute
        ),
        month_key: run.timestamp.month_key(),
        commit: run.git_revision.commit.clone(),
        git_dirty: run.git_revision.dirty,
        system_uuid: run.system.system_uuid.clone(),
        public_system_id: run.system.public_system_id.clone(),
        display_name: run.system.display_name.clone(),
        warmup_runs: run.warmup_runs,
        measured_iterations: run.measured_iterations,
        suite_kind: run.suite_kind.persisted_name().to_string(),
        primary_metric_name: run.suite_kind.primary_metric_name().to_string(),
        suite_average_ms: run.suite.average_ms,
        suite_case_spread_ms: run.suite.case_spread_ms,
        thread_count: run.thread_count,
        groups: run
            .groups
            .iter()
            .map(|group| LocalGroupRecord {
                name: group.group_name.clone(),
                case_count: group.case_count,
                average_ms: group.average_ms,
            })
            .collect(),
        cases: run.cases.iter().map(local_case_from_result).collect(),
    }
}

fn local_case_from_result(case: &BenchmarkCaseResult) -> LocalCaseRecord {
    LocalCaseRecord {
        case_id: case.case_id.clone(),
        workload_id: case.identity.as_ref().map(|id| id.workload_id.clone()),
        source_fingerprint: case
            .identity
            .as_ref()
            .map(|id| id.source_fingerprint.clone()),
        measurement_fingerprint: case
            .identity
            .as_ref()
            .map(|id| id.measurement_fingerprint.clone()),
        group_name: case.group_name.clone(),
        runner: case.runner.clone(),
        mean_ms: case.mean_ms,
        median_ms: case.median_ms,
        stddev_ms: case.stddev_ms,
        stage_timings: case
            .observations
            .stage_timings
            .iter()
            .map(local_metric_from_benchmark_metric)
            .collect(),
        counters: case
            .observations
            .counters
            .iter()
            .map(local_metric_from_benchmark_metric)
            .collect(),
    }
}

/// Convert a persisted record into comparison/report case results.
pub fn to_case_results(record: &LocalRunRecord) -> Vec<BenchmarkCaseResult> {
    record
        .cases
        .iter()
        .map(|case| BenchmarkCaseResult {
            case_id: case.case_id.clone(),
            identity: build_identity_from_record(case),
            group_name: case.group_name.clone(),
            runner: case.runner.clone(),
            mean_ms: case.mean_ms,
            median_ms: case.median_ms,
            stddev_ms: case.stddev_ms,
            observations: BenchmarkCaseObservations {
                stage_timings: case
                    .stage_timings
                    .iter()
                    .map(benchmark_metric_from_local_metric)
                    .collect(),
                counters: case
                    .counters
                    .iter()
                    .map(benchmark_metric_from_local_metric)
                    .collect(),
            },
        })
        .collect()
}

pub fn to_group_stats(record: &LocalRunRecord) -> Vec<BenchmarkGroupStats> {
    record
        .groups
        .iter()
        .map(|group| BenchmarkGroupStats {
            group_name: group.name.clone(),
            case_count: group.case_count,
            average_ms: group.average_ms,
        })
        .collect()
}

fn local_metric_from_benchmark_metric(metric: &BenchmarkMetric) -> LocalMetricRecord {
    LocalMetricRecord {
        name: metric.name.clone(),
        value: metric.value,
    }
}

fn benchmark_metric_from_local_metric(metric: &LocalMetricRecord) -> BenchmarkMetric {
    BenchmarkMetric {
        name: metric.name.clone(),
        value: metric.value,
    }
}

/// Build typed identity from a persisted record, returning None for legacy
/// records that lack source or measurement fingerprints.
fn build_identity_from_record(
    case: &LocalCaseRecord,
) -> Option<crate::bench_types::BenchmarkMeasurementIdentity> {
    use crate::bench_types::BenchmarkMeasurementIdentity;

    let workload_id = case.workload_id.clone()?;
    let source_fingerprint = case.source_fingerprint.clone()?;
    let measurement_fingerprint = case.measurement_fingerprint.clone()?;

    if workload_id.is_empty() || source_fingerprint.is_empty() || measurement_fingerprint.is_empty()
    {
        return None;
    }

    Some(BenchmarkMeasurementIdentity {
        workload_id,
        source_fingerprint,
        measurement_fingerprint,
    })
}

// ------------------------
//  Legacy format adapters
// ------------------------

/// V6 on-disk shape. V6 used a single mixed `workload_fingerprint` field that
/// combined source bytes with runner declarations. It must never be relabeled
/// as a source fingerprint; the adapter converts it to the current shape with
/// `None` identity so comparisons skip these records as incomparable.
#[derive(Debug, Deserialize)]
struct LegacyV6Run {
    format_version: u32,
    benchmark_protocol_version: u32,
    timestamp: String,
    month_key: String,
    commit: Option<String>,
    git_dirty: Option<bool>,
    system_uuid: String,
    public_system_id: String,
    display_name: String,
    warmup_runs: usize,
    measured_iterations: usize,
    suite_kind: String,
    primary_metric_name: String,
    suite_average_ms: f64,
    suite_case_spread_ms: f64,
    thread_count: Option<u32>,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LegacyV6Case>,
}

#[derive(Debug, Deserialize)]
struct LegacyV6Case {
    case_id: String,
    workload_id: Option<String>,
    workload_fingerprint: Option<String>,
    group_name: String,
    runner: BenchmarkRunner,
    mean_ms: f64,
    median_ms: f64,
    stddev_ms: f64,
    stage_timings: Vec<LocalMetricRecord>,
    counters: Vec<LocalMetricRecord>,
}

fn adapt_v6(legacy: LegacyV6Run) -> Result<LocalRunRecord, String> {
    let cases = legacy
        .cases
        .into_iter()
        .map(|case| LocalCaseRecord {
            case_id: case.case_id,
            workload_id: case.workload_id,
            source_fingerprint: None,
            measurement_fingerprint: None,
            group_name: case.group_name,
            runner: case.runner,
            mean_ms: case.mean_ms,
            median_ms: case.median_ms,
            stddev_ms: case.stddev_ms,
            stage_timings: case.stage_timings,
            counters: case.counters,
        })
        .collect();

    Ok(LocalRunRecord {
        format_version: 6,
        benchmark_protocol_version: legacy.benchmark_protocol_version,
        timestamp: legacy.timestamp,
        month_key: legacy.month_key,
        commit: legacy.commit,
        git_dirty: legacy.git_dirty,
        system_uuid: legacy.system_uuid,
        public_system_id: legacy.public_system_id,
        display_name: legacy.display_name,
        warmup_runs: legacy.warmup_runs,
        measured_iterations: legacy.measured_iterations,
        suite_kind: legacy.suite_kind,
        primary_metric_name: legacy.primary_metric_name,
        suite_average_ms: legacy.suite_average_ms,
        suite_case_spread_ms: legacy.suite_case_spread_ms,
        thread_count: legacy.thread_count,
        groups: legacy.groups,
        cases,
    })
}

#[derive(Debug, Deserialize)]
struct LegacyCommonRun {
    timestamp: String,
    month_key: String,
    commit: Option<String>,
    system_uuid: String,
    public_system_id: String,
    display_name: String,
    #[serde(default = "default_warmup_runs")]
    warmup_runs: usize,
    #[serde(default = "default_measured_iterations")]
    measured_iterations: usize,
}

#[derive(Debug, Deserialize)]
struct LegacyV1Run {
    format_version: u32,
    #[serde(flatten)]
    common: LegacyCommonRun,
    #[serde(default)]
    suite_mean_ms: f64,
    #[serde(default)]
    suite_stddev_ms: f64,
    cases: Vec<LegacyV1Case>,
}

#[derive(Debug, Deserialize)]
struct LegacyV1Case {
    name: String,
    command: String,
    args: Vec<String>,
    #[serde(default)]
    mean_ms: f64,
    #[serde(default)]
    stddev_ms: f64,
}

#[derive(Debug, Deserialize)]
struct LegacyV2Run {
    format_version: u32,
    #[serde(flatten)]
    common: LegacyCommonRun,
    #[serde(default)]
    suite_average_ms: f64,
    #[serde(default)]
    suite_case_spread_ms: f64,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LegacyGroupedCase>,
}

#[derive(Debug, Deserialize)]
struct LegacyV3Run {
    format_version: u32,
    #[serde(flatten)]
    common: LegacyCommonRun,
    #[serde(default)]
    suite_average_ms: f64,
    #[serde(default)]
    suite_case_spread_ms: f64,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LegacyGroupedCase>,
}

#[derive(Debug, Deserialize)]
struct LegacyV4Run {
    format_version: u32,
    #[serde(flatten)]
    common: LegacyCommonRun,
    #[serde(default = "default_suite_kind")]
    suite_kind: String,
    primary_metric_name: Option<String>,
    #[serde(default)]
    suite_average_ms: f64,
    #[serde(default)]
    suite_case_spread_ms: f64,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LegacyGroupedCase>,
}

#[derive(Debug, Deserialize)]
struct LegacyV5Run {
    format_version: u32,
    #[serde(flatten)]
    common: LegacyCommonRun,
    #[serde(default = "default_suite_kind")]
    suite_kind: String,
    primary_metric_name: Option<String>,
    #[serde(default)]
    suite_average_ms: f64,
    #[serde(default)]
    suite_case_spread_ms: f64,
    thread_count: Option<u32>,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LegacyGroupedCase>,
}

#[derive(Debug, Deserialize)]
struct LegacyGroupedCase {
    name: String,
    group_name: Option<String>,
    command: String,
    args: Vec<String>,
    #[serde(default)]
    mean_ms: f64,
    median_ms: Option<f64>,
    #[serde(default)]
    stddev_ms: f64,
    #[serde(default)]
    stage_timings: Vec<LocalMetricRecord>,
    #[serde(default)]
    counters: Vec<LocalMetricRecord>,
}

fn adapt_v1(legacy: LegacyV1Run) -> Result<LocalRunRecord, String> {
    let cases: Vec<LocalCaseRecord> = legacy
        .cases
        .into_iter()
        .map(|case| {
            let group_name = infer_legacy_group_name(&case.name, &case.command, &case.args);
            legacy_case(LegacyCaseData {
                case_id: case.name,
                group_name,
                command: case.command,
                args: case.args,
                mean_ms: case.mean_ms,
                median_ms: case.mean_ms,
                stddev_ms: case.stddev_ms,
                stage_timings: Vec::new(),
                counters: Vec::new(),
            })
        })
        .collect::<Result<_, _>>()?;
    let groups = local_group_records_from_cases(&cases);

    Ok(legacy_record(LegacyRecordData {
        format_version: legacy.format_version,
        common: legacy.common,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: "wall_time_ms".to_string(),
        suite_average_ms: legacy.suite_mean_ms,
        suite_case_spread_ms: legacy.suite_stddev_ms,
        thread_count: None,
        groups,
        cases,
    }))
}

fn adapt_v2(legacy: LegacyV2Run) -> Result<LocalRunRecord, String> {
    adapt_grouped_legacy(LegacyGroupedRunData {
        format_version: legacy.format_version,
        common: legacy.common,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: None,
        suite_average_ms: legacy.suite_average_ms,
        suite_case_spread_ms: legacy.suite_case_spread_ms,
        thread_count: None,
        groups: legacy.groups,
        cases: legacy.cases,
    })
}

fn adapt_v3(legacy: LegacyV3Run) -> Result<LocalRunRecord, String> {
    adapt_grouped_legacy(LegacyGroupedRunData {
        format_version: legacy.format_version,
        common: legacy.common,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: None,
        suite_average_ms: legacy.suite_average_ms,
        suite_case_spread_ms: legacy.suite_case_spread_ms,
        thread_count: None,
        groups: legacy.groups,
        cases: legacy.cases,
    })
}

fn adapt_v4(legacy: LegacyV4Run) -> Result<LocalRunRecord, String> {
    adapt_grouped_legacy(LegacyGroupedRunData {
        format_version: legacy.format_version,
        common: legacy.common,
        suite_kind: legacy.suite_kind,
        primary_metric_name: legacy.primary_metric_name,
        suite_average_ms: legacy.suite_average_ms,
        suite_case_spread_ms: legacy.suite_case_spread_ms,
        thread_count: None,
        groups: legacy.groups,
        cases: legacy.cases,
    })
}

fn adapt_v5(legacy: LegacyV5Run) -> Result<LocalRunRecord, String> {
    adapt_grouped_legacy(LegacyGroupedRunData {
        format_version: legacy.format_version,
        common: legacy.common,
        suite_kind: legacy.suite_kind,
        primary_metric_name: legacy.primary_metric_name,
        suite_average_ms: legacy.suite_average_ms,
        suite_case_spread_ms: legacy.suite_case_spread_ms,
        thread_count: legacy.thread_count,
        groups: legacy.groups,
        cases: legacy.cases,
    })
}

struct LegacyGroupedRunData {
    format_version: u32,
    common: LegacyCommonRun,
    suite_kind: String,
    primary_metric_name: Option<String>,
    suite_average_ms: f64,
    suite_case_spread_ms: f64,
    thread_count: Option<u32>,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LegacyGroupedCase>,
}

fn adapt_grouped_legacy(data: LegacyGroupedRunData) -> Result<LocalRunRecord, String> {
    let cases = data
        .cases
        .into_iter()
        .map(|case| {
            let group_name = case
                .group_name
                .unwrap_or_else(|| infer_legacy_group_name(&case.name, &case.command, &case.args));
            let median_ms = case.median_ms.unwrap_or(case.mean_ms);

            legacy_case(LegacyCaseData {
                case_id: case.name,
                group_name,
                command: case.command,
                args: case.args,
                mean_ms: case.mean_ms,
                median_ms,
                stddev_ms: case.stddev_ms,
                stage_timings: case.stage_timings,
                counters: case.counters,
            })
        })
        .collect::<Result<_, _>>()?;
    let primary_metric_name = data
        .primary_metric_name
        .unwrap_or_else(|| default_primary_metric_name(&data.suite_kind));

    Ok(legacy_record(LegacyRecordData {
        format_version: data.format_version,
        common: data.common,
        suite_kind: data.suite_kind,
        primary_metric_name,
        suite_average_ms: data.suite_average_ms,
        suite_case_spread_ms: data.suite_case_spread_ms,
        thread_count: data.thread_count,
        groups: data.groups,
        cases,
    }))
}

struct LegacyRecordData {
    format_version: u32,
    common: LegacyCommonRun,
    suite_kind: String,
    primary_metric_name: String,
    suite_average_ms: f64,
    suite_case_spread_ms: f64,
    thread_count: Option<u32>,
    groups: Vec<LocalGroupRecord>,
    cases: Vec<LocalCaseRecord>,
}

fn legacy_record(data: LegacyRecordData) -> LocalRunRecord {
    LocalRunRecord {
        format_version: data.format_version,
        benchmark_protocol_version: 0,
        timestamp: data.common.timestamp,
        month_key: data.common.month_key,
        commit: data.common.commit,
        git_dirty: None,
        system_uuid: data.common.system_uuid,
        public_system_id: data.common.public_system_id,
        display_name: data.common.display_name,
        warmup_runs: data.common.warmup_runs,
        measured_iterations: data.common.measured_iterations,
        suite_kind: data.suite_kind,
        primary_metric_name: data.primary_metric_name,
        suite_average_ms: data.suite_average_ms,
        suite_case_spread_ms: data.suite_case_spread_ms,
        thread_count: data.thread_count,
        groups: data.groups,
        cases: data.cases,
    }
}

struct LegacyCaseData {
    case_id: String,
    group_name: String,
    command: String,
    args: Vec<String>,
    mean_ms: f64,
    median_ms: f64,
    stddev_ms: f64,
    stage_timings: Vec<LocalMetricRecord>,
    counters: Vec<LocalMetricRecord>,
}

fn legacy_case(data: LegacyCaseData) -> Result<LocalCaseRecord, String> {
    let runner = match data.command.as_str() {
        "check" => BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: data.args,
        },
        "build" => BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Build,
            args: data.args,
        },
        "frontend" => BenchmarkRunner::Frontend {
            profile: FrontendBenchmarkProfile::Dev,
        },
        _ => {
            return Err(format!(
                "legacy case '{}' has unknown command '{}'",
                data.case_id, data.command
            ));
        }
    };

    Ok(LocalCaseRecord {
        case_id: data.case_id,
        workload_id: None,
        source_fingerprint: None,
        measurement_fingerprint: None,
        group_name: data.group_name,
        runner,
        mean_ms: data.mean_ms,
        median_ms: data.median_ms,
        stddev_ms: data.stddev_ms,
        stage_timings: data.stage_timings,
        counters: data.counters,
    })
}

fn default_warmup_runs() -> usize {
    1
}

fn default_measured_iterations() -> usize {
    10
}

fn default_suite_kind() -> String {
    "end_to_end_cli".to_string()
}

fn default_primary_metric_name(suite_kind: &str) -> String {
    BenchmarkSuiteKind::from_persisted_name(suite_kind)
        .map_or("wall_time_ms", |kind| kind.primary_metric_name())
        .to_string()
}

fn infer_legacy_group_name(name: &str, command: &str, args: &[String]) -> String {
    let mut text = format!("{name} {command}");
    for argument in args {
        text.push(' ');
        text.push_str(argument);
    }

    if text.contains("speed-test.moth") || text.contains("speed-test.bst") {
        "core".to_string()
    } else if args.iter().any(|argument| argument == "docs") {
        "docs".to_string()
    } else if text.contains("template-stress")
        || text.contains("type-stress")
        || text.contains("fold-stress")
        || text.contains("pattern-stress")
        || text.contains("collection-stress")
    {
        "stress".to_string()
    } else if text.contains("module-graph") {
        "module".to_string()
    } else if text.contains("borrow-stress") {
        "borrow".to_string()
    } else {
        "ungrouped".to_string()
    }
}

fn local_group_records_from_cases(cases: &[LocalCaseRecord]) -> Vec<LocalGroupRecord> {
    let benchmark_cases: Vec<BenchmarkCaseResult> = cases
        .iter()
        .map(|case| BenchmarkCaseResult {
            case_id: case.case_id.clone(),
            identity: None,
            group_name: case.group_name.clone(),
            runner: case.runner.clone(),
            mean_ms: case.mean_ms,
            median_ms: case.median_ms,
            stddev_ms: case.stddev_ms,
            observations: BenchmarkCaseObservations::default(),
        })
        .collect();

    crate::bench_types::calculate_group_stats(&benchmark_cases)
        .into_iter()
        .map(|group| LocalGroupRecord {
            name: group.group_name,
            case_count: group.case_count,
            average_ms: group.average_ms,
        })
        .collect()
}

#[cfg(test)]
mod tests;
