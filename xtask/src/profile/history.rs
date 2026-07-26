//! Profile run history storage and retrieval.
//!
//! WHAT: Provides append-only JSONL storage for compact derived profile
//! metadata under `benchmarks/local-data/profile-runs.jsonl`. Each record
//! captures one full profiling run's metadata: run identity, system info,
//! filter mode, sample rate, and per-case hotspot/observation summaries.
//!
//! WHY: Storing derived profile history enables drift comparison between
//! runs on the same system without re-parsing raw Samply profiles. The
//! JSONL format matches `bench_history::runs.jsonl` style: one record per
//! line, append-only, with `format_version` for forward compatibility.
//!
//! # What this module owns
//! - `ProfileHistoryRecord` and related structs for JSONL schema
//! - `append_profile_run()` to write one record after a successful run
//! - `read_profile_runs()` to load all records for drift comparison
//! - Profile-owned JSON serialization and one explicit v1 identity adapter
//!
//! # What this module does NOT own
//! - Drift detection and reporting (see `drift.rs`)
//! - Profile JSON parsing or hotspot extraction (see `parse.rs`, `hotspots.rs`)
//! - Agent summaries and enriched per-case summaries (see `summary.rs`)

use crate::bench_system::{SystemIdentityMode, load_or_create_system};
use crate::bench_types::BenchmarkMetric;
use std::fs;
use std::path::Path;

use super::json::escape;

/// Path to the profile history file, relative to repo root.
pub const PROFILE_RUNS_JSONL_PATH: &str = "benchmarks/local-data/profile-runs.jsonl";

/// Current on-disk format version for profile history records.
const HISTORY_FORMAT_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
//  Data model
// ---------------------------------------------------------------------------

/// A complete profile run record stored in JSONL history.
///
/// WHAT: Captures one profiling run's identity, system, filter mode,
/// sample rate, and per-case derived metadata (observations, hotspots).
///
/// WHY: A single run-level record keeps the JSONL file compact and
/// makes drift comparison straightforward: find the latest previous
/// record matching system/case/filter/rate, then compare per-case data.
#[derive(Debug, Clone)]
pub struct ProfileHistoryRecord {
    /// Schema version for forward compatibility.
    pub format_version: u32,
    /// Run identifier (e.g., "2026-06-18T10-30-abc1234").
    pub run_id: String,
    /// ISO-style timestamp string.
    pub timestamp: String,
    /// Short commit hash, if available.
    pub commit: Option<String>,
    /// Stable system UUID from `benchmarks/local-data/system.toml`.
    pub system_uuid: String,
    /// Human-readable system display name.
    pub system_display: String,
    /// Filter mode label ("terse", "normal", "deep", "raw-index").
    pub filter_mode: String,
    /// Samply sampling rate in Hz, if explicitly set.
    pub sample_rate_hz: Option<f64>,
    /// Per-case derived metadata.
    pub cases: Vec<HistoryCaseRecord>,
}

/// Per-case derived metadata within a profile history record.
///
/// WHAT: Stores the observation data and hotspot summary for one case
/// so that drift comparison can access wall time, stage timings,
/// counters, and hot functions without re-parsing raw profiles.
///
/// WHY: One record per case keeps the history file self-contained
/// and avoids coupling drift comparison to the filesystem layout.
#[derive(Debug, Clone)]
pub struct HistoryCaseRecord {
    /// Authored case ID from the typed benchmark manifest.
    pub case_id: String,
    /// Group name for the case.
    pub group_name: String,
    /// The command executed (e.g., "check", "build").
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Observation pass wall time in milliseconds.
    pub observation_wall_ms: f64,
    /// Total sample count from the Samply profile.
    pub sample_count: usize,
    /// Total sample weight from the Samply profile.
    pub sample_weight: f64,
    /// Stage timings from the observation pass.
    pub stage_timings: Vec<BenchmarkMetric>,
    /// Counters from the observation pass.
    pub counters: Vec<BenchmarkMetric>,
    /// Hot functions with inclusive/self samples and percentages.
    pub hot_functions: Vec<HistoryHotFunction>,
    /// Top bucket label for the hottest function.
    pub top_bucket_label: String,
    /// Relative path to the run directory from repo root.
    pub run_directory_path: String,
}

/// A hot function entry within a history case record.
///
/// WHAT: Stores the minimal data needed for drift comparison: function
/// name, bucket, inclusive/self samples and percentages.
///
/// WHY: Drift comparison only needs percentages and sample counts;
/// callers, callees, and estimated milliseconds are derived during
/// comparison rather than stored.
#[derive(Debug, Clone)]
pub struct HistoryHotFunction {
    /// Resolved function name from the profile.
    pub name: String,
    /// Owner bucket label (e.g., "AST", "Tokenization", "std").
    pub bucket_label: String,
    /// Inclusive sample weight.
    pub inclusive_samples: f64,
    /// Self sample weight.
    pub self_samples: f64,
    /// Inclusive percentage of total sample weight.
    pub inclusive_pct: f64,
    /// Self percentage of total sample weight.
    pub self_pct: f64,
}

// ---------------------------------------------------------------------------
//  Public entry points
// ---------------------------------------------------------------------------

/// Append one profile run record to the history JSONL file.
///
/// WHAT: Writes a single JSON line to `benchmarks/local-data/profile-runs.jsonl`,
/// creating the file and parent directory if they do not exist.
///
/// WHY: Append-only writes keep the history file safe for concurrent
/// reads and avoid corrupting previous records. The format matches
/// `bench_history::append_local_run()` style.
pub fn append_profile_run(path: &Path, record: &ProfileHistoryRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create profile history directory '{}': {}",
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
        .map_err(|e| format!("Failed to open profile-runs.jsonl: {}", e))?;

    use std::io::Write;
    writeln!(file, "{}", line).map_err(|e| format!("Failed to append to profile-runs.jsonl: {}", e))
}

/// Read all profile run records from the history JSONL file.
///
/// WHAT: Loads every line from the JSONL file, skipping malformed lines
/// with warnings (matching `bench_history::read_local_runs()` style).
///
/// WHY: Drift comparison needs the full history to find the latest
/// comparable previous record. Malformed lines are skipped rather
/// than failing the entire read so old data does not block new runs.
pub fn read_profile_runs(path: &Path) -> Result<Vec<ProfileHistoryRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read profile-runs.jsonl: {}", e))?;

    let mut records = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Some(format_version) = extract_u32_field(trimmed, "format_version") else {
            eprintln!(
                "Warning: skipping malformed profile-runs.jsonl line: missing format_version"
            );
            continue;
        };

        if format_version > HISTORY_FORMAT_VERSION {
            continue;
        }

        match parse_jsonl_record(trimmed) {
            Ok(record) => records.push(record),
            Err(e) => {
                eprintln!("Warning: skipping malformed profile-runs.jsonl line: {}", e);
            }
        }
    }

    Ok(records)
}

/// Build a `ProfileHistoryRecord` from a completed profiling run.
///
/// WHAT: Assembles the run identity, system info, filter mode, sample rate,
/// and per-case data into a single record ready for JSONL append.
///
/// WHY: The orchestrator calls this after all cases complete successfully,
/// passing the data it already accumulated during the run. System info
/// is loaded from `benchmarks/local-data/system.toml` to keep the record
/// self-contained for drift comparison.
pub fn build_history_record(
    run_id: &str,
    timestamp: &str,
    commit: Option<&str>,
    filter_mode: &str,
    sample_rate_hz: Option<f64>,
    cases: Vec<HistoryCaseRecord>,
) -> Result<ProfileHistoryRecord, String> {
    let system = load_or_create_system(SystemIdentityMode::ReadOnly)?;
    let (system_uuid, system_display) = match system {
        Some(s) => (s.system_uuid, s.display_name),
        None => ("unknown".to_string(), "unknown".to_string()),
    };

    Ok(ProfileHistoryRecord {
        format_version: HISTORY_FORMAT_VERSION,
        run_id: run_id.to_string(),
        timestamp: timestamp.to_string(),
        commit: commit.map(|s| s.to_string()),
        system_uuid,
        system_display,
        filter_mode: filter_mode.to_string(),
        sample_rate_hz,
        cases,
    })
}

// ---------------------------------------------------------------------------
//  Profile JSON serialization
// ---------------------------------------------------------------------------

/// Serialize a `ProfileHistoryRecord` to a single JSONL line.
///
/// WHAT: Produces compact, valid JSON using manual formatting to match
/// the profile-local schema.
fn format_record_as_jsonl(record: &ProfileHistoryRecord) -> String {
    let mut parts = Vec::new();

    parts.push(format!(r#""format_version":{}"#, record.format_version));
    parts.push(format!(r#""run_id":"{}""#, escape(&record.run_id)));
    parts.push(format!(r#""timestamp":"{}""#, escape(&record.timestamp)));

    match &record.commit {
        Some(c) => parts.push(format!(r#""commit":"{}""#, escape(c))),
        None => parts.push(r#""commit":null"#.to_string()),
    }

    parts.push(format!(
        r#""system_uuid":"{}""#,
        escape(&record.system_uuid)
    ));
    parts.push(format!(
        r#""system_display":"{}""#,
        escape(&record.system_display)
    ));
    parts.push(format!(
        r#""filter_mode":"{}""#,
        escape(&record.filter_mode)
    ));

    match record.sample_rate_hz {
        Some(rate) => parts.push(format!(r#""sample_rate_hz":{}"#, rate)),
        None => parts.push(r#""sample_rate_hz":null"#.to_string()),
    }

    let cases_json: Vec<String> = record.cases.iter().map(format_case_record_json).collect();
    parts.push(format!(r#""cases":[{}]"#, cases_json.join(",")));

    format!("{{{}}}", parts.join(","))
}

/// Format a single `HistoryCaseRecord` as a JSON object string.
fn format_case_record_json(case: &HistoryCaseRecord) -> String {
    let mut parts = Vec::new();

    parts.push(format!(r#""case_id":"{}""#, escape(&case.case_id)));
    parts.push(format!(r#""group_name":"{}""#, escape(&case.group_name)));
    parts.push(format!(r#""command":"{}""#, escape(&case.command)));

    let args_json: Vec<String> = case
        .args
        .iter()
        .map(|a| format!(r#""{}""#, escape(a)))
        .collect();
    parts.push(format!(r#""args":[{}]"#, args_json.join(",")));

    parts.push(format!(
        r#""observation_wall_ms":{}"#,
        case.observation_wall_ms
    ));
    parts.push(format!(r#""sample_count":{}"#, case.sample_count));
    parts.push(format!(r#""sample_weight":{}"#, case.sample_weight));

    let stages_json: Vec<String> = case.stage_timings.iter().map(format_metric_json).collect();
    parts.push(format!(r#""stage_timings":[{}]"#, stages_json.join(",")));

    let counters_json: Vec<String> = case.counters.iter().map(format_metric_json).collect();
    parts.push(format!(r#""counters":[{}]"#, counters_json.join(",")));

    let functions_json: Vec<String> = case
        .hot_functions
        .iter()
        .map(format_hot_function_json)
        .collect();
    parts.push(format!(r#""hot_functions":[{}]"#, functions_json.join(",")));

    parts.push(format!(
        r#""top_bucket_label":"{}""#,
        escape(&case.top_bucket_label)
    ));
    parts.push(format!(
        r#""run_directory_path":"{}""#,
        escape(&case.run_directory_path)
    ));

    format!("{{{}}}", parts.join(","))
}

/// Format a single `BenchmarkMetric` as a JSON object string.
fn format_metric_json(metric: &BenchmarkMetric) -> String {
    format!(
        r#"{{"name":"{}","value":{}}}"#,
        escape(&metric.name),
        metric.value
    )
}

/// Format a single `HistoryHotFunction` as a JSON object string.
fn format_hot_function_json(func: &HistoryHotFunction) -> String {
    format!(
        r#"{{"name":"{}","bucket_label":"{}","inclusive_samples":{},"self_samples":{},"inclusive_pct":{},"self_pct":{}}}"#,
        escape(&func.name),
        escape(&func.bucket_label),
        func.inclusive_samples,
        func.self_samples,
        func.inclusive_pct,
        func.self_pct,
    )
}

// ---------------------------------------------------------------------------
//  Profile JSON parsing
// ---------------------------------------------------------------------------

/// Parse a single JSONL line into a `ProfileHistoryRecord`.
fn parse_jsonl_record(line: &str) -> Result<ProfileHistoryRecord, String> {
    let format_version =
        extract_u32_field(line, "format_version").ok_or("missing format_version")?;

    let run_id = extract_string_field(line, "run_id").ok_or("missing run_id")?;
    let timestamp = extract_string_field(line, "timestamp").ok_or("missing timestamp")?;
    let commit = extract_string_field(line, "commit");
    let system_uuid =
        extract_string_field(line, "system_uuid").unwrap_or_else(|| "unknown".to_string());
    let system_display =
        extract_string_field(line, "system_display").unwrap_or_else(|| "unknown".to_string());
    let filter_mode =
        extract_string_field(line, "filter_mode").unwrap_or_else(|| "terse".to_string());
    let sample_rate_hz = extract_f64_field(line, "sample_rate_hz");

    let cases = match format_version {
        1 => extract_cases_array(line, parse_legacy_v1_case_object)?,
        HISTORY_FORMAT_VERSION => extract_cases_array(line, parse_case_object)?,
        _ => {
            return Err(format!(
                "unsupported profile history format_version {format_version}"
            ));
        }
    };

    Ok(ProfileHistoryRecord {
        format_version,
        run_id,
        timestamp,
        commit,
        system_uuid,
        system_display,
        filter_mode,
        sample_rate_hz,
        cases,
    })
}

/// Extract the "cases" array from a JSON object line.
fn extract_cases_array(
    line: &str,
    parse_case: fn(&str) -> Result<HistoryCaseRecord, String>,
) -> Result<Vec<HistoryCaseRecord>, String> {
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
        .map(|object| parse_case(&object))
        .collect()
}

/// Parse a single case JSON object into a `HistoryCaseRecord`.
fn parse_case_object(obj: &str) -> Result<HistoryCaseRecord, String> {
    let case_id = extract_string_field(obj, "case_id").ok_or("case missing case_id")?;
    parse_case_fields(obj, case_id)
}

/// Adapt the sole profile-history v1 identity field into the current domain.
fn parse_legacy_v1_case_object(obj: &str) -> Result<HistoryCaseRecord, String> {
    let legacy_case_name =
        extract_string_field(obj, "case_name").ok_or("legacy v1 case missing case_name")?;
    parse_case_fields(obj, legacy_case_name)
}

fn parse_case_fields(obj: &str, case_id: String) -> Result<HistoryCaseRecord, String> {
    let group_name =
        extract_string_field(obj, "group_name").unwrap_or_else(|| "ungrouped".to_string());
    let command = extract_string_field(obj, "command").ok_or("case missing command")?;
    let args = extract_string_array(obj, "args").unwrap_or_default();
    let observation_wall_ms = extract_f64_field(obj, "observation_wall_ms").unwrap_or(0.0);
    let sample_count = extract_usize_field(obj, "sample_count").unwrap_or(0);
    let sample_weight = extract_f64_field(obj, "sample_weight").unwrap_or(0.0);
    let stage_timings = extract_metric_array(obj, "stage_timings").unwrap_or_default();
    let counters = extract_metric_array(obj, "counters").unwrap_or_default();
    let hot_functions = extract_hot_functions_array(obj).unwrap_or_default();
    let top_bucket_label =
        extract_string_field(obj, "top_bucket_label").unwrap_or_else(|| "unknown".to_string());
    let run_directory_path = extract_string_field(obj, "run_directory_path").unwrap_or_default();

    Ok(HistoryCaseRecord {
        case_id,
        group_name,
        command,
        args,
        observation_wall_ms,
        sample_count,
        sample_weight,
        stage_timings,
        counters,
        hot_functions,
        top_bucket_label,
        run_directory_path,
    })
}

/// Extract the "hot_functions" array from a JSON object.
fn extract_hot_functions_array(obj: &str) -> Result<Vec<HistoryHotFunction>, String> {
    let key = r#""hot_functions":"#;
    let Some(start) = obj.find(key).and_then(|index| index.checked_add(key.len())) else {
        return Ok(Vec::new());
    };
    let rest = &obj[start..];

    extract_object_array_items(rest, "hot_functions")?
        .into_iter()
        .map(|object| parse_hot_function_object(&object))
        .collect()
}

/// Parse a single hot function JSON object.
fn parse_hot_function_object(obj: &str) -> Result<HistoryHotFunction, String> {
    let name = extract_string_field(obj, "name").ok_or("hot function missing name")?;
    let bucket_label =
        extract_string_field(obj, "bucket_label").unwrap_or_else(|| "unknown".to_string());
    let inclusive_samples = extract_f64_field(obj, "inclusive_samples").unwrap_or(0.0);
    let self_samples = extract_f64_field(obj, "self_samples").unwrap_or(0.0);
    let inclusive_pct = extract_f64_field(obj, "inclusive_pct").unwrap_or(0.0);
    let self_pct = extract_f64_field(obj, "self_pct").unwrap_or(0.0);

    Ok(HistoryHotFunction {
        name,
        bucket_label,
        inclusive_samples,
        self_samples,
        inclusive_pct,
        self_pct,
    })
}

/// Extract a metric array from a JSON object.
fn extract_metric_array(obj: &str, field: &str) -> Result<Vec<BenchmarkMetric>, String> {
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
            Ok(BenchmarkMetric { name, value })
        })
        .collect()
}

// ---------------------------------------------------------------------------
//  Profile JSON field extraction helpers
// ---------------------------------------------------------------------------

/// Extract a quoted string field value from a JSON object line.
fn extract_string_field(line: &str, field: &str) -> Option<String> {
    let key = format!(r#""{}":"#, field);
    let start = line.find(&key)? + key.len();
    let rest = &line[start..];

    let mut idx = 0;
    while idx < rest.len() && rest.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    if rest.as_bytes().get(idx) != Some(&b'"') {
        if rest[idx..].starts_with("null") {
            return None;
        }
        return None;
    }
    idx += 1;

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

/// Extract an array of JSON objects from a JSON value.
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

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
