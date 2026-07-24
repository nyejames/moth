//! Local raw benchmark history
//!
//! This module provides read/write access to benchmarks/local-data/runs.jsonl,
//! a local-only line-delimited JSON file that stores enough history to compare
//! future runs from the same system. Public monthly summaries are lossy by design;
//! this file preserves per-case detail without committing raw data.
//!
//! WHAT: Appends one JSON object per successful bench run; reads back for
//!       previous-run comparison and monthly summary generation.
//! WHY:  Keeps detailed history local-only while still enabling per-system
//!       delta calculation and long-term trend inspection.

use crate::bench_types::{
    BenchmarkCaseObservations, BenchmarkCaseResult, BenchmarkGroupStats, BenchmarkMetric,
    BenchmarkRun, BenchmarkSuiteKind,
};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Path to the local raw benchmark history file, relative to repo root.
pub const RUNS_JSONL_PATH: &str = "benchmarks/local-data/runs.jsonl";

/// Current on-disk format version. Bumped when the JSONL schema changes.
const FORMAT_VERSION: u32 = 5;

/// A single benchmark run as stored in runs.jsonl.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalRunRecord {
    pub format_version: u32,
    pub timestamp: String,
    pub month_key: String,
    pub commit: Option<String>,
    pub system_uuid: String,
    pub public_system_id: String,
    pub display_name: String,
    pub warmup_runs: usize,
    pub measured_iterations: usize,
    pub suite_kind: String,
    pub primary_metric_name: String,
    pub suite_average_ms: f64,
    pub suite_case_spread_ms: f64,
    /// Effective RAYON_NUM_THREADS: None for default threads, Some(n) for a fixed count.
    ///
    /// Old records (format version <= 4) do not carry this field and are
    /// treated as None (default-thread identity) during parsing.
    pub thread_count: Option<u32>,
    pub groups: Vec<LocalGroupRecord>,
    pub cases: Vec<LocalCaseRecord>,
}

/// Aggregated group stats within a stored run.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalGroupRecord {
    pub name: String,
    pub case_count: usize,
    pub average_ms: f64,
}

/// A single case result within a stored run.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalCaseRecord {
    pub name: String,
    pub group_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub stddev_ms: f64,
    pub stage_timings: Vec<LocalMetricRecord>,
    pub counters: Vec<LocalMetricRecord>,
}

/// A local-only named detailed timer or counter value.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalMetricRecord {
    pub name: String,
    pub value: f64,
}

/// Read all runs from runs.jsonl.
///
/// Returns an empty vector if the file does not exist.
/// Skips lines that fail to parse or have an unknown format_version.
pub fn read_local_runs(path: &Path) -> Result<Vec<LocalRunRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents =
        fs::read_to_string(path).map_err(|e| format!("Failed to read runs.jsonl: {}", e))?;

    let mut runs = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(format_version) = extract_u32_field(trimmed, "format_version") else {
            eprintln!("Warning: skipping malformed runs.jsonl line: missing format_version");
            continue;
        };

        if format_version > FORMAT_VERSION {
            continue;
        }

        match parse_jsonl_line(trimmed) {
            Ok(record) => runs.push(record),
            Err(e) => {
                // Skip malformed lines rather than failing the entire file
                eprintln!("Warning: skipping malformed runs.jsonl line: {}", e);
            }
        }
    }

    Ok(runs)
}

/// Find the most recent run matching the system, suite kind, and thread identity.
///
/// Scans from the end so the latest appended record wins. Filtering by
/// `thread_count` ensures default-thread and fixed-thread runs never compare
/// against each other, and different fixed counts never match. Old records
/// without `thread_count` are treated as `None` (default).
pub fn find_latest_matching_run<'a>(
    runs: &'a [LocalRunRecord],
    system_uuid: &str,
    suite_kind: BenchmarkSuiteKind,
    thread_count: Option<u32>,
) -> Option<&'a LocalRunRecord> {
    let persisted_suite_kind = suite_kind.persisted_name();

    runs.iter().rfind(|r| {
        r.system_uuid == system_uuid
            && r.suite_kind == persisted_suite_kind
            && r.thread_count == thread_count
    })
}

/// Append a single run record to runs.jsonl.
///
/// Creates the file (and parent directory) if missing.
pub fn append_local_run(path: &Path, record: &LocalRunRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create local-data directory '{}': {}",
                parent.display(),
                e
            )
        })?;
    }

    let line = format_record_as_jsonl(record);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Failed to open runs.jsonl: {}", e))?;

    use std::io::Write;
    writeln!(file, "{}", line).map_err(|e| format!("Failed to append to runs.jsonl: {}", e))
}

/// Get the short commit hash of the current HEAD.
///
/// Returns None if git is unavailable or the repo has no commits.
pub fn get_commit_hash() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let hash = String::from_utf8_lossy(&output.stdout);
    let trimmed = hash.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Capture the effective `RAYON_NUM_THREADS` setting as a normalized
/// thread identity.
///
/// Returns `Ok(Some(n))` for a valid positive integer, `Ok(None)` when the
/// variable is unset (Rayon default threads), or `Err` when the variable is
/// set to an empty string, zero, a non-numeric value, or a non-Unicode value.
///
/// Only an unset variable means default. A non-Unicode value must surface as a
/// clear tooling error instead of silently collapsing into default identity.
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

/// Format a thread identity for display.
///
/// Returns "default" for `None` and "fixed: N" for `Some(n)`. A fixed-thread
/// run must never look like a default run in any output surface.
pub fn thread_identity_label(thread_count: Option<u32>) -> String {
    match thread_count {
        None => "default".to_string(),
        Some(count) => format!("fixed: {count}"),
    }
}

/// Format the inline thread-identity suffix for a benchmark result line.
///
/// Returns an empty string for default threads and ` [threads: fixed: N]` for
/// a fixed thread count. A fixed run must always be visibly distinct from
/// default in any stdout surface.
pub fn thread_identity_suffix(thread_count: Option<u32>) -> String {
    match thread_count {
        None => String::new(),
        Some(_) => format!(" [threads: {}]", thread_identity_label(thread_count)),
    }
}

/// Parse a `RAYON_NUM_THREADS` value into a normalized thread identity.
///
/// Accepts a positive integer as `Some(n)`. Rejects empty, zero, or
/// non-numeric values with a clear error message so invalid values never
/// silently become default.
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

/// Convert a BenchmarkRun into a LocalRunRecord for persistence.
pub fn to_local_record(run: &BenchmarkRun, commit: Option<String>) -> LocalRunRecord {
    debug_assert_eq!(
        run.groups
            .iter()
            .map(|group| group.case_count)
            .sum::<usize>(),
        run.cases.len()
    );

    let cases = run
        .cases
        .iter()
        .map(|c| LocalCaseRecord {
            name: c.case_name.clone(),
            group_name: c.group_name.clone(),
            command: c.command.clone(),
            args: c.args.clone(),
            mean_ms: c.mean_ms,
            median_ms: c.median_ms,
            stddev_ms: c.stddev_ms,
            stage_timings: c
                .observations
                .stage_timings
                .iter()
                .map(local_metric_from_benchmark_metric)
                .collect(),
            counters: c
                .observations
                .counters
                .iter()
                .map(local_metric_from_benchmark_metric)
                .collect(),
        })
        .collect();
    let groups = run
        .groups
        .iter()
        .map(|group| LocalGroupRecord {
            name: group.group_name.clone(),
            case_count: group.case_count,
            average_ms: group.average_ms,
        })
        .collect();

    LocalRunRecord {
        format_version: FORMAT_VERSION,
        timestamp: format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            run.timestamp.year,
            run.timestamp.month,
            run.timestamp.day,
            run.timestamp.hour,
            run.timestamp.minute
        ),
        month_key: run.timestamp.month_key(),
        commit,
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
        groups,
        cases,
    }
}

/// Convert a LocalRunRecord into case results for comparison.
pub fn to_case_results(record: &LocalRunRecord) -> Vec<BenchmarkCaseResult> {
    record
        .cases
        .iter()
        .map(|c| BenchmarkCaseResult {
            case_name: c.name.clone(),
            group_name: c.group_name.clone(),
            command: c.command.clone(),
            args: c.args.clone(),
            mean_ms: c.mean_ms,
            median_ms: c.median_ms,
            stddev_ms: c.stddev_ms,
            observations: BenchmarkCaseObservations {
                stage_timings: c
                    .stage_timings
                    .iter()
                    .map(benchmark_metric_from_local_metric)
                    .collect(),
                counters: c
                    .counters
                    .iter()
                    .map(benchmark_metric_from_local_metric)
                    .collect(),
            },
        })
        .collect()
}

/// Convert a LocalRunRecord into group stats for summary rendering.
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

// ---------------------------------------------------------------------------
// Manual JSON formatting (std-only)
// ---------------------------------------------------------------------------

/// Serialize a LocalRunRecord to a single JSONL line.
///
/// Produces compact, valid JSON without external dependencies; xtask is kept
/// std-only and the schema is small enough for manual formatting.
pub fn format_record_as_jsonl(record: &LocalRunRecord) -> String {
    let mut parts = Vec::new();

    parts.push(format!(r#""format_version":{}"#, record.format_version));
    parts.push(format!(
        r#""timestamp":"{}""#,
        json_escape(&record.timestamp)
    ));
    parts.push(format!(
        r#""month_key":"{}""#,
        json_escape(&record.month_key)
    ));

    match &record.commit {
        Some(c) => parts.push(format!(r#""commit":"{}""#, json_escape(c))),
        None => parts.push(r#""commit":null"#.to_string()),
    }

    parts.push(format!(
        r#""system_uuid":"{}""#,
        json_escape(&record.system_uuid)
    ));
    parts.push(format!(
        r#""public_system_id":"{}""#,
        json_escape(&record.public_system_id)
    ));
    parts.push(format!(
        r#""display_name":"{}""#,
        json_escape(&record.display_name)
    ));
    parts.push(format!(r#""warmup_runs":{}"#, record.warmup_runs));
    parts.push(format!(
        r#""measured_iterations":{}"#,
        record.measured_iterations
    ));
    parts.push(format!(
        r#""suite_kind":"{}""#,
        json_escape(&record.suite_kind)
    ));
    parts.push(format!(
        r#""primary_metric_name":"{}""#,
        json_escape(&record.primary_metric_name)
    ));
    parts.push(format!(r#""suite_average_ms":{}"#, record.suite_average_ms));
    parts.push(format!(
        r#""suite_case_spread_ms":{}"#,
        record.suite_case_spread_ms
    ));
    match record.thread_count {
        Some(count) => parts.push(format!(r#""thread_count":{}"#, count)),
        None => parts.push(r#""thread_count":null"#.to_string()),
    }

    let group_parts: Vec<String> = record
        .groups
        .iter()
        .map(|group| {
            format!(
                r#"{{"name":"{}","case_count":{},"average_ms":{}}}"#,
                json_escape(&group.name),
                group.case_count,
                group.average_ms
            )
        })
        .collect();

    parts.push(format!(r#""groups":[{}]"#, group_parts.join(",")));

    let case_parts: Vec<String> = record
        .cases
        .iter()
        .map(|c| {
            let arg_list = c
                .args
                .iter()
                .map(|a| format!(r#""{}""#, json_escape(a)))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                r#"{{"name":"{}","group_name":"{}","command":"{}","args":[{}],"mean_ms":{},"median_ms":{},"stddev_ms":{},"stage_timings":[{}],"counters":[{}]}}"#,
                json_escape(&c.name),
                json_escape(&c.group_name),
                json_escape(&c.command),
                arg_list,
                c.mean_ms,
                c.median_ms,
                c.stddev_ms,
                format_metric_array(&c.stage_timings),
                format_metric_array(&c.counters)
            )
        })
        .collect();

    parts.push(format!(r#""cases":[{}]"#, case_parts.join(",")));

    format!("{{{}}}", parts.join(","))
}

/// Escape a string for JSON output.
///
/// WHAT: Escapes backslash, double-quote, and common control characters.
/// WHY:  Prevents malformed JSON when benchmark names or paths contain
///       special characters.
pub fn json_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result
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

fn format_metric_array(metrics: &[LocalMetricRecord]) -> String {
    metrics
        .iter()
        .map(|metric| {
            format!(
                r#"{{"name":"{}","value":{}}}"#,
                json_escape(&metric.name),
                metric.value
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// Manual JSON parsing (std-only)
// ---------------------------------------------------------------------------

/// Parse a single JSONL line into a LocalRunRecord.
///
/// WHAT: Minimal field-extraction parser for the known benchmark JSONL schema.
/// WHY:  Keeps xtask std-only; the schema is small and fully controlled.
fn parse_jsonl_line(line: &str) -> Result<LocalRunRecord, String> {
    let format_version =
        extract_u32_field(line, "format_version").ok_or("missing format_version")?;

    match format_version {
        1 => parse_v1_record(line),
        2..=5 => parse_grouped_record(line),
        _ => Err(format!("unsupported format_version {format_version}")),
    }
}

fn parse_common_record_fields(line: &str) -> Result<LocalRecordFields, String> {
    let timestamp = extract_string_field(line, "timestamp").ok_or("missing timestamp")?;
    let month_key = extract_string_field(line, "month_key").ok_or("missing month_key")?;
    let system_uuid = extract_string_field(line, "system_uuid").ok_or("missing system_uuid")?;
    let public_system_id =
        extract_string_field(line, "public_system_id").ok_or("missing public_system_id")?;
    let display_name = extract_string_field(line, "display_name").ok_or("missing display_name")?;

    let commit = extract_string_field(line, "commit");

    let warmup_runs = extract_usize_field(line, "warmup_runs").unwrap_or(1);
    let measured_iterations = extract_usize_field(line, "measured_iterations").unwrap_or(10);

    Ok(LocalRecordFields {
        timestamp,
        month_key,
        commit,
        system_uuid,
        public_system_id,
        display_name,
        warmup_runs,
        measured_iterations,
    })
}

fn parse_v1_record(line: &str) -> Result<LocalRunRecord, String> {
    let fields = parse_common_record_fields(line)?;
    let suite_mean_ms = extract_f64_field(line, "suite_mean_ms").unwrap_or(0.0);
    let suite_stddev_ms = extract_f64_field(line, "suite_stddev_ms").unwrap_or(0.0);
    let cases = extract_cases_array(line)?;
    let groups = local_group_records_from_cases(&cases);

    Ok(LocalRunRecord {
        format_version: 1,
        timestamp: fields.timestamp,
        month_key: fields.month_key,
        commit: fields.commit,
        system_uuid: fields.system_uuid,
        public_system_id: fields.public_system_id,
        display_name: fields.display_name,
        warmup_runs: fields.warmup_runs,
        measured_iterations: fields.measured_iterations,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: "wall_time_ms".to_string(),
        suite_average_ms: suite_mean_ms,
        suite_case_spread_ms: suite_stddev_ms,
        thread_count: None,
        groups,
        cases,
    })
}

fn parse_grouped_record(line: &str) -> Result<LocalRunRecord, String> {
    let fields = parse_common_record_fields(line)?;
    let suite_average_ms = extract_f64_field(line, "suite_average_ms").unwrap_or(0.0);
    let suite_case_spread_ms = extract_f64_field(line, "suite_case_spread_ms").unwrap_or(0.0);
    let cases = extract_cases_array(line)?;
    let groups = extract_groups_array(line)?;

    let suite_kind =
        extract_string_field(line, "suite_kind").unwrap_or_else(|| "end_to_end_cli".to_string());
    let default_primary_metric_name = BenchmarkSuiteKind::from_persisted_name(&suite_kind)
        .map_or("wall_time_ms", |suite_kind| {
            suite_kind.primary_metric_name()
        });
    let primary_metric_name = extract_string_field(line, "primary_metric_name")
        .unwrap_or_else(|| default_primary_metric_name.to_string());

    // Old records (format version <= 4) do not carry thread_count; treat
    // absent or null as None (default-thread identity).
    let thread_count = extract_u32_field(line, "thread_count");

    Ok(LocalRunRecord {
        format_version: extract_u32_field(line, "format_version").unwrap_or(2),
        timestamp: fields.timestamp,
        month_key: fields.month_key,
        commit: fields.commit,
        system_uuid: fields.system_uuid,
        public_system_id: fields.public_system_id,
        display_name: fields.display_name,
        warmup_runs: fields.warmup_runs,
        measured_iterations: fields.measured_iterations,
        suite_kind,
        primary_metric_name,
        suite_average_ms,
        suite_case_spread_ms,
        thread_count,
        groups,
        cases,
    })
}

struct LocalRecordFields {
    timestamp: String,
    month_key: String,
    commit: Option<String>,
    system_uuid: String,
    public_system_id: String,
    display_name: String,
    warmup_runs: usize,
    measured_iterations: usize,
}

/// Extract a quoted string field value from a JSON object line.
fn extract_string_field(line: &str, field: &str) -> Option<String> {
    let key = format!(r#""{}":"#, field);
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];

    // Skip whitespace
    let mut idx = 0;
    while idx < rest.len() && rest.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    if rest.as_bytes().get(idx) != Some(&b'"') {
        // Could be null
        if rest[idx..].starts_with("null") {
            return None;
        }
        return None;
    }
    idx += 1; // skip opening quote

    let mut result = String::new();
    let mut escaped = false;

    while idx < rest.len() {
        let ch = rest.as_bytes()[idx] as char;
        if escaped {
            match ch {
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                'n' => result.push('\n'),
                'r' => result.push('\r'),
                't' => result.push('\t'),
                'u' => {
                    // \uXXXX
                    let hex_start = idx + 1;
                    let hex_end = (hex_start + 4).min(rest.len());
                    let hex = &rest[hex_start..hex_end];
                    if let Ok(code) = u32::from_str_radix(hex, 16)
                        && let Some(c) = char::from_u32(code)
                    {
                        result.push(c);
                    }
                    idx = hex_end - 1;
                }
                c => result.push(c),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            break;
        } else {
            result.push(ch);
        }
        idx += 1;
    }

    Some(result)
}

/// Extract an unsigned integer field value from a JSON object line.
fn extract_usize_field(line: &str, field: &str) -> Option<usize> {
    let key = format!(r#""{}":"#, field);
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];

    // Skip whitespace
    let mut idx = 0;
    while idx < rest.len() && rest.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    let end = rest[idx..]
        .find([',', '}', ']'])
        .unwrap_or(rest.len() - idx);

    let num_str = rest[idx..idx + end].trim();
    num_str.parse().ok()
}

/// Extract a u32 field value from a JSON object line.
fn extract_u32_field(line: &str, field: &str) -> Option<u32> {
    extract_usize_field(line, field).map(|v| v as u32)
}

/// Extract an f64 field value from a JSON object line.
fn extract_f64_field(line: &str, field: &str) -> Option<f64> {
    let key = format!(r#""{}":"#, field);
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];

    // Skip whitespace
    let mut idx = 0;
    while idx < rest.len() && rest.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    let end = rest[idx..]
        .find([',', '}', ']'])
        .unwrap_or(rest.len() - idx);

    let num_str = rest[idx..idx + end].trim();
    num_str.parse().ok()
}

/// Extract the "groups" array from a JSON object line.
fn extract_groups_array(line: &str) -> Result<Vec<LocalGroupRecord>, String> {
    let key = r#""groups":"#;
    let start = line
        .find(key)
        .ok_or("missing groups field")?
        .checked_add(key.len())
        .ok_or("invalid groups position")?;
    let rest = &line[start..];

    let group_objects = extract_object_array_items(rest, "groups")?;
    group_objects
        .into_iter()
        .map(|object| parse_group_object(&object))
        .collect()
}

fn parse_group_object(obj: &str) -> Result<LocalGroupRecord, String> {
    let name = extract_string_field(obj, "name").ok_or("group missing name")?;
    let case_count = extract_usize_field(obj, "case_count").unwrap_or(0);
    let average_ms = extract_f64_field(obj, "average_ms").unwrap_or(0.0);

    Ok(LocalGroupRecord {
        name,
        case_count,
        average_ms,
    })
}

/// Extract the "cases" array from a JSON object line.
fn extract_cases_array(line: &str) -> Result<Vec<LocalCaseRecord>, String> {
    let key = r#""cases":"#;
    let start = line
        .find(key)
        .ok_or("missing cases field")?
        .checked_add(key.len())
        .ok_or("invalid cases position")?;
    let rest = &line[start..];

    let case_objects = extract_object_array_items(rest, "cases")?;
    case_objects
        .into_iter()
        .map(|object| parse_case_object(&object))
        .collect()
}

fn extract_object_array_items(rest: &str, field: &str) -> Result<Vec<String>, String> {
    let mut idx = 0;
    while idx < rest.len() && rest.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    if rest.as_bytes().get(idx) != Some(&b'[') {
        return Err(format!("{field} field is not an array"));
    }
    idx += 1;

    let mut objects = Vec::new();
    let mut brace_depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut obj_start: Option<usize> = None;

    while idx < rest.len() {
        let ch = rest.as_bytes()[idx] as char;

        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            in_string = !in_string;
        } else if !in_string {
            match ch {
                '{' => {
                    if brace_depth == 0 {
                        obj_start = Some(idx);
                    }
                    brace_depth += 1;
                }
                '}' => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        if let Some(start) = obj_start {
                            objects.push(rest[start..=idx].to_string());
                        }
                        obj_start = None;
                    }
                }
                ']' if brace_depth == 0 => {
                    break;
                }
                _ => {}
            }
        }
        idx += 1;
    }

    Ok(objects)
}

/// Parse a single case object string into a LocalCaseRecord.
fn parse_case_object(obj: &str) -> Result<LocalCaseRecord, String> {
    let name = extract_string_field(obj, "name").ok_or("case missing name")?;
    let command = extract_string_field(obj, "command").ok_or("case missing command")?;
    let mean_ms = extract_f64_field(obj, "mean_ms").unwrap_or(0.0);
    let median_ms = extract_f64_field(obj, "median_ms").unwrap_or(mean_ms);
    let stddev_ms = extract_f64_field(obj, "stddev_ms").unwrap_or(0.0);

    let args = extract_string_array(obj, "args")?;
    let group_name = extract_string_field(obj, "group_name")
        .unwrap_or_else(|| infer_legacy_group_name(&name, &command, &args));

    Ok(LocalCaseRecord {
        name,
        group_name,
        command,
        args,
        mean_ms,
        median_ms,
        stddev_ms,
        stage_timings: extract_metric_array(obj, "stage_timings")?,
        counters: extract_metric_array(obj, "counters")?,
    })
}

fn extract_metric_array(obj: &str, field: &str) -> Result<Vec<LocalMetricRecord>, String> {
    let key = format!(r#""{}":"#, field);
    let Some(start) = obj
        .find(&key)
        .and_then(|index| index.checked_add(key.len()))
    else {
        return Ok(Vec::new());
    };
    let rest = &obj[start..];

    extract_object_array_items(rest, field)?
        .into_iter()
        .map(|object| {
            let name = extract_string_field(&object, "name")
                .ok_or_else(|| format!("{field} metric missing name"))?;
            let value = extract_f64_field(&object, "value").unwrap_or(0.0);

            Ok(LocalMetricRecord { name, value })
        })
        .collect()
}

fn infer_legacy_group_name(name: &str, command: &str, args: &[String]) -> String {
    let mut text = String::new();
    text.push_str(name);
    text.push(' ');
    text.push_str(command);
    for arg in args {
        text.push(' ');
        text.push_str(arg);
    }

    if text.contains("speed-test.moth") {
        "core".to_string()
    } else if args.iter().any(|arg| arg == "docs") {
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
            case_name: case.name.clone(),
            group_name: case.group_name.clone(),
            command: case.command.clone(),
            args: case.args.clone(),
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

/// Extract an array of strings from a JSON object.
fn extract_string_array(obj: &str, field: &str) -> Result<Vec<String>, String> {
    let key = format!(r#""{}":"#, field);
    let start = obj
        .find(&key)
        .ok_or(format!("missing {} field", field))?
        .checked_add(key.len())
        .ok_or("invalid array position")?;
    let rest = &obj[start..];

    let mut idx = 0;
    while idx < rest.len() && rest.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    if rest.as_bytes().get(idx) != Some(&b'[') {
        return Err(format!("{} field is not an array", field));
    }
    idx += 1;

    let mut items = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut current = String::new();

    while idx < rest.len() {
        let ch = rest.as_bytes()[idx] as char;

        if escaped {
            match ch {
                '"' => current.push('"'),
                '\\' => current.push('\\'),
                'n' => current.push('\n'),
                'r' => current.push('\r'),
                't' => current.push('\t'),
                _ => current.push(ch),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            if in_string {
                items.push(current.clone());
                current.clear();
                in_string = false;
            } else {
                in_string = true;
            }
        } else if ch == ']' && !in_string {
            break;
        } else if in_string {
            current.push(ch);
        }
        idx += 1;
    }

    Ok(items)
}

#[cfg(test)]
mod tests;
