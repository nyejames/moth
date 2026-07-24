//! Agent summaries and report formatting for the profiling workflow.
//!
//! WHAT: Generates `agent-summary.md` (root), `profile-hotspots.json` (root),
//! and enriched per-case `summary.md` files that combine observation data,
//! hotspot extraction results, and deterministic investigation hints.
//!
//! WHY: Agents should not need to open every per-case file. The root summary
//! ranks cases by a combined signal score and surfaces the most actionable
//! profiling data: top stages, top functions, owner buckets, source paths,
//! and short hints about where to investigate.
//!
//! # What this module owns
//! - `CaseSummaryData` for assembling per-case summary inputs
//! - `RootProfileHotspots` for the root `profile-hotspots.json` schema
//! - Deterministic hint generation from stage/bucket/signal patterns
//! - Markdown formatting for `agent-summary.md` and per-case `summary.md`
//! - Writing root summary artifacts to the run directory
//!
//! # What this module does NOT own
//! - Profile JSON parsing (see `parse.rs`)
//! - Hotspot extraction and filtering (see `hotspots.rs`)
//! - Owner bucket definitions (see `buckets.rs`)
//! - Artifact directory layout (see `artifacts.rs`)
//! - Profile history or drift (Phase 6)

use std::fs;

use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::BenchmarkMetric;

use super::artifacts::ProfileRunPaths;
use super::hotspots::HotspotExtractionResult;
use super::observations::ProfileObservation;
use super::options::ProfileFilterMode;

// ---------------------------------------------------------------------------
//  Data types
// ---------------------------------------------------------------------------

/// Assembled data for one case's enriched summary.
///
/// WHAT: Combines the observation pass data, hotspot extraction result,
/// and path metadata needed to generate both the per-case summary.md
/// and the case's contribution to root artifacts.
///
/// WHY: A named struct avoids threading many fields through formatting
/// functions and makes the summary's data dependencies explicit.
pub(crate) struct CaseSummaryData<'a> {
    /// Observation data from the non-profiled pass.
    pub(crate) observation: &'a ProfileObservation,
    /// Hotspot extraction result from the parsed Samply profile.
    pub(crate) hotspots: &'a HotspotExtractionResult,
    /// Relative path to the raw Samply profile from the run root.
    pub(crate) profile_relative_path: String,
    /// Filter mode used for this run.
    pub(crate) filter: ProfileFilterMode,
}

/// Root-level profile hotspots JSON schema.
///
/// WHAT: Machine-readable aggregation of per-case hotspot data with
/// run metadata. Written as `profile-hotspots.json` in the run root.
///
/// WHY: Agents and tools can read a single JSON file to understand
/// the profiling run without scanning per-case directories.
#[derive(Debug)]
pub(crate) struct RootProfileHotspots {
    pub(crate) format_version: u32,
    pub(crate) run_id: String,
    pub(crate) timestamp: String,
    pub(crate) commit: Option<String>,
    pub(crate) filter: String,
    pub(crate) samply_rate_hz: Option<f64>,
    pub(crate) case_count: usize,
    pub(crate) cases: Vec<RootCaseHotspots>,
}

/// Per-case data within the root hotspots JSON.
#[derive(Debug)]
pub(crate) struct RootCaseHotspots {
    pub(crate) case_name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) observation_wall_ms: f64,
    pub(crate) top_stage_timings: Vec<BenchmarkMetric>,
    pub(crate) top_counters: Vec<BenchmarkMetric>,
    pub(crate) hot_functions: Vec<RootHotFunction>,
    pub(crate) bucket_summary: Vec<BucketSummaryEntry>,
    pub(crate) warnings: Vec<String>,
    pub(crate) symbolication_status: String,
    pub(crate) raw_address_function_count: usize,
    pub(crate) hot_function_count: usize,
    pub(crate) profile_path: String,
    pub(crate) summary_path: String,
}

/// A hot function within the root hotspots JSON.
#[derive(Debug)]
pub(crate) struct RootHotFunction {
    pub(crate) name: String,
    pub(crate) bucket_label: String,
    pub(crate) inclusive_pct: f64,
    pub(crate) self_pct: f64,
    pub(crate) estimated_inclusive_ms: f64,
    pub(crate) estimated_self_ms: f64,
}

/// A bucket summary entry aggregating function counts per owner bucket.
#[derive(Debug)]
pub(crate) struct BucketSummaryEntry {
    pub(crate) label: String,
    pub(crate) function_count: usize,
    pub(crate) total_inclusive_pct: f64,
    pub(crate) suggested_paths: Vec<String>,
}

// ---------------------------------------------------------------------------
//  Constants
// ---------------------------------------------------------------------------

/// Current format version for summary artifacts.
const SUMMARY_FORMAT_VERSION: u32 = 1;

/// Maximum number of stage timings to show in root summary case entries.
const ROOT_TOP_STAGES: usize = 3;

/// Maximum number of counters to show in root summary case entries.
const ROOT_TOP_COUNTERS: usize = 2;

/// Maximum number of hot functions to show in the root hotspots JSON per case.
const ROOT_HOT_FUNCTION_LIMIT: usize = 10;

// ---------------------------------------------------------------------------
//  Public entry points
// ---------------------------------------------------------------------------

/// Generate an enriched per-case `summary.md`.
///
/// WHAT: Combines observation data (wall time, stages, counters) with
/// hotspot data (hot functions, buckets, hints) into a single readable
/// summary. Written to the case's `summary.md`.
///
/// WHY: The enriched summary gives agents a complete per-case picture
/// without opening the raw profile, hotspots.json, and observations.json
/// separately.
pub(crate) fn generate_case_summary(
    run_paths: &ProfileRunPaths,
    data: &CaseSummaryData<'_>,
) -> Result<(), String> {
    let md = format_enriched_case_summary(data, run_paths);

    let case_paths = run_paths.case_paths(&data.observation.case_name);
    fs::write(&case_paths.summary_md, md).map_err(|e| {
        format!(
            "Failed to write enriched summary.md '{}': {}",
            case_paths.summary_md.display(),
            e
        )
    })
}

/// Generate the root `profile-hotspots.json`.
///
/// WHAT: Aggregates per-case hotspot data into a single JSON file with
/// run metadata, per-case commands, wall times, top stages, top counters,
/// hot functions, bucket summaries, warnings, and relative paths.
///
/// WHY: A structured JSON file lets tools and agents consume profiling
/// data without parsing markdown or scanning directories.
pub(crate) fn generate_root_hotspots_json(
    run_paths: &ProfileRunPaths,
    cases: &[CaseSummaryData<'_>],
    run_id: &str,
    commit: Option<&str>,
    filter: ProfileFilterMode,
    samply_rate_hz: Option<f64>,
) -> Result<(), String> {
    let root = build_root_hotspots(cases, run_id, commit, filter, samply_rate_hz);
    let json = format_root_hotspots_json(&root);

    let path = run_paths.root.join("profile-hotspots.json");
    fs::write(&path, json).map_err(|e| {
        format!(
            "Failed to write profile-hotspots.json '{}': {}",
            path.display(),
            e
        )
    })
}

/// Generate the root `agent-summary.md`.
///
/// WHAT: Produces a terse, ranked markdown summary of the profiling run
/// sorted by a combined signal score. Each case entry includes wall time,
/// top stage, top function, bucket, source paths, profile path, and a
/// deterministic hint.
///
/// WHY: Agents read this file first to decide which case and source area
/// to investigate, without opening every per-case file.
pub(crate) fn generate_agent_summary(
    run_paths: &ProfileRunPaths,
    cases: &[CaseSummaryData<'_>],
    run_id: &str,
    filter: ProfileFilterMode,
) -> Result<(), String> {
    let md = format_agent_summary_md(cases, run_id, filter);

    let path = run_paths.root.join("agent-summary.md");
    fs::write(&path, md).map_err(|e| {
        format!(
            "Failed to write agent-summary.md '{}': {}",
            path.display(),
            e
        )
    })
}

/// Append a drift section to the existing `agent-summary.md`.
///
/// WHAT: Reads the current `agent-summary.md`, appends the drift summary
/// section, and writes the file back.
///
/// WHY: The drift section belongs in the agent summary so agents can see
/// profiling movement without opening a separate file. Appending keeps
/// the summary generation logic clean and drift-awareness in the orchestrator.
pub(crate) fn append_drift_to_agent_summary(
    run_paths: &ProfileRunPaths,
    drift_section: &str,
) -> Result<(), String> {
    let path = run_paths.root.join("agent-summary.md");
    let existing = fs::read_to_string(&path).map_err(|e| {
        format!(
            "Failed to read agent-summary.md '{}': {}",
            path.display(),
            e
        )
    })?;

    let updated = format!("{}\n{}", existing.trim_end(), drift_section);
    fs::write(&path, updated).map_err(|e| {
        format!(
            "Failed to write agent-summary.md '{}': {}",
            path.display(),
            e
        )
    })
}

// ---------------------------------------------------------------------------
//  Root hotspots JSON
// ---------------------------------------------------------------------------

/// Build the root hotspots data structure from per-case data.
fn build_root_hotspots(
    cases: &[CaseSummaryData<'_>],
    run_id: &str,
    commit: Option<&str>,
    filter: ProfileFilterMode,
    samply_rate_hz: Option<f64>,
) -> RootProfileHotspots {
    let case_entries: Vec<RootCaseHotspots> = cases
        .iter()
        .map(|data| build_root_case_hotspots(data))
        .collect();

    RootProfileHotspots {
        format_version: SUMMARY_FORMAT_VERSION,
        run_id: run_id.to_string(),
        timestamp: BenchmarkTimestamp::now().format_run_header(),
        commit: commit.map(|s| s.to_string()),
        filter: filter.display_label().to_string(),
        samply_rate_hz,
        case_count: cases.len(),
        cases: case_entries,
    }
}

/// Build per-case root hotspots from a case summary data entry.
fn build_root_case_hotspots(data: &CaseSummaryData<'_>) -> RootCaseHotspots {
    let obs = data.observation;
    let hotspots = data.hotspots;

    // Top stage timings: sort by value descending, take top N.
    let mut stages = obs.observations.stage_timings.clone();
    stages.sort_by(|a, b| b.value.total_cmp(&a.value));
    stages.truncate(ROOT_TOP_STAGES);

    // Top counters: sort by value descending, take top N.
    let mut counters = obs.observations.counters.clone();
    counters.sort_by(|a, b| b.value.total_cmp(&a.value));
    counters.truncate(ROOT_TOP_COUNTERS);

    // Hot functions: convert to the root JSON shape.
    let hot_functions: Vec<RootHotFunction> = hotspots
        .functions
        .iter()
        .take(ROOT_HOT_FUNCTION_LIMIT)
        .map(|func| RootHotFunction {
            name: func.name.clone(),
            bucket_label: func.bucket.label.clone(),
            inclusive_pct: round_2dp(func.inclusive_pct),
            self_pct: round_2dp(func.self_pct),
            estimated_inclusive_ms: round_2dp(func.estimated_inclusive_ms),
            estimated_self_ms: round_2dp(func.estimated_self_ms),
        })
        .collect();

    // Bucket summary: aggregate hot functions by bucket label.
    let bucket_summary = build_bucket_summary(&hotspots.functions);

    RootCaseHotspots {
        case_name: obs.case_name.clone(),
        command: obs.command.clone(),
        args: obs.command_args.clone(),
        observation_wall_ms: round_2dp(obs.wall_ms),
        top_stage_timings: stages,
        top_counters: counters,
        hot_functions,
        bucket_summary,
        warnings: hotspots.warnings.clone(),
        symbolication_status: hotspots.symbolication.status.as_str().to_string(),
        raw_address_function_count: hotspots.symbolication.raw_address_function_count,
        hot_function_count: hotspots.symbolication.hot_function_count,
        profile_path: data.profile_relative_path.clone(),
        summary_path: format!("cases/{}/summary.md", obs.case_name),
    }
}

/// Aggregate hot functions by bucket label into summary entries.
fn build_bucket_summary(
    functions: &[super::hotspots::ProfileHotFunction],
) -> Vec<BucketSummaryEntry> {
    use std::collections::BTreeMap;

    let mut by_label: BTreeMap<String, BucketSummaryEntry> = BTreeMap::new();

    for func in functions {
        let entry = by_label
            .entry(func.bucket.label.clone())
            .or_insert_with(|| BucketSummaryEntry {
                label: func.bucket.label.clone(),
                function_count: 0,
                total_inclusive_pct: 0.0,
                suggested_paths: func.bucket.suggested_paths.clone(),
            });

        entry.function_count += 1;
        entry.total_inclusive_pct += func.inclusive_pct;
    }

    // Sort by total inclusive percentage descending.
    let mut entries: Vec<BucketSummaryEntry> = by_label.into_values().collect();
    entries.sort_by(|a, b| b.total_inclusive_pct.total_cmp(&a.total_inclusive_pct));
    entries
}

/// Format root hotspots as serde_json.
fn format_root_hotspots_json(root: &RootProfileHotspots) -> String {
    let cases_json: Vec<serde_json::Value> = root
        .cases
        .iter()
        .map(|case| {
            let args_json: Vec<serde_json::Value> =
                case.args.iter().map(|a| serde_json::json!(a)).collect();

            let stages_json: Vec<serde_json::Value> = case
                .top_stage_timings
                .iter()
                .map(|m| serde_json::json!({"name": m.name, "value": round_2dp(m.value)}))
                .collect();

            let counters_json: Vec<serde_json::Value> = case
                .top_counters
                .iter()
                .map(|m| serde_json::json!({"name": m.name, "value": round_2dp(m.value)}))
                .collect();

            let functions_json: Vec<serde_json::Value> = case
                .hot_functions
                .iter()
                .map(|func| {
                    serde_json::json!({
                        "name": func.name,
                        "bucket": func.bucket_label,
                        "inclusive_pct": func.inclusive_pct,
                        "self_pct": func.self_pct,
                        "estimated_inclusive_ms": func.estimated_inclusive_ms,
                        "estimated_self_ms": func.estimated_self_ms,
                    })
                })
                .collect();

            let buckets_json: Vec<serde_json::Value> = case
                .bucket_summary
                .iter()
                .map(|bucket| {
                    serde_json::json!({
                        "label": bucket.label,
                        "function_count": bucket.function_count,
                        "total_inclusive_pct": round_2dp(bucket.total_inclusive_pct),
                        "suggested_paths": bucket.suggested_paths,
                    })
                })
                .collect();

            serde_json::json!({
                "case_name": case.case_name,
                "command": case.command,
                "args": args_json,
                "observation_wall_ms": case.observation_wall_ms,
                "top_stage_timings": stages_json,
                "top_counters": counters_json,
                "hot_functions": functions_json,
                "bucket_summary": buckets_json,
                "warnings": case.warnings,
                "symbolication": {
                    "status": case.symbolication_status,
                    "raw_address_function_count": case.raw_address_function_count,
                    "hot_function_count": case.hot_function_count,
                },
                "profile_path": case.profile_path,
                "summary_path": case.summary_path,
            })
        })
        .collect();

    let output = serde_json::json!({
        "format_version": root.format_version,
        "run_id": root.run_id,
        "timestamp": root.timestamp,
        "commit": root.commit,
        "filter": root.filter,
        "samply_rate_hz": root.samply_rate_hz,
        "case_count": root.case_count,
        "cases": cases_json,
    });

    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

// ---------------------------------------------------------------------------
//  Agent summary markdown
// ---------------------------------------------------------------------------

/// Format the root `agent-summary.md`.
fn format_agent_summary_md(
    cases: &[CaseSummaryData<'_>],
    run_id: &str,
    filter: ProfileFilterMode,
) -> String {
    let mut lines = Vec::new();

    // Header
    lines.push("# Profiling agent summary".to_string());
    lines.push(String::new());
    lines.push(format!("Run: {}", run_id));
    lines.push(format!("Filter: {}", filter.display_label()));
    lines.push(format!("Cases: {}", cases.len()));
    lines.push(String::new());

    if cases.is_empty() {
        lines.push("No cases profiled.".to_string());
        return lines.join("\n");
    }

    // Sort cases by combined signal score (highest first).
    let mut scored_cases: Vec<(f64, &CaseSummaryData<'_>)> = cases
        .iter()
        .map(|data| (combined_signal_score(data), data))
        .collect();
    scored_cases.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Apply the root case limit.
    let limit = filter.root_case_limit();
    let selected: Vec<_> = scored_cases.into_iter().take(limit).collect();

    lines.push("## Strongest signals".to_string());
    lines.push(String::new());

    for (score, data) in &selected {
        append_agent_case_entry(&mut lines, data, *score);
    }

    // If there are more cases than the limit, note them.
    if cases.len() > limit {
        lines.push(format!(
            "_{} additional cases omitted (filter: {}). Use `deep` to see all._",
            cases.len() - limit,
            filter.display_label(),
        ));
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Append one case entry to the agent summary markdown.
fn append_agent_case_entry(lines: &mut Vec<String>, data: &CaseSummaryData<'_>, _score: f64) {
    let obs = data.observation;
    let hotspots = data.hotspots;

    // Case heading
    lines.push(format!("### {}", obs.case_name));
    lines.push(String::new());

    // Command
    lines.push(format!(
        "- Command: `{} {}`",
        obs.command,
        obs.command_args.join(" ")
    ));

    // Wall time
    lines.push(format!("- Wall: ~{:.0}ms", obs.wall_ms));

    // Top stage timing
    if let Some(top_stage) = obs
        .observations
        .stage_timings
        .iter()
        .max_by(|a, b| a.value.total_cmp(&b.value))
    {
        lines.push(format!(
            "- Top stage: {} ~{:.0}ms",
            top_stage.name, top_stage.value
        ));
    }

    // Top hot function
    if hotspots.symbolication.is_failed() {
        lines.push(format!(
            "- Symbolication: failed ({}/{} hot functions are raw addresses)",
            hotspots.symbolication.raw_address_function_count,
            hotspots.symbolication.hot_function_count
        ));
        lines.push("- Top function: unavailable (raw addresses only)".to_string());
        lines.push("- Bucket: unavailable".to_string());
    } else if let Some(top_func) = hotspots.functions.first() {
        let display_name = truncate_function_name(&top_func.name, 80);
        lines.push(format!("- Top function: `{}`", display_name));
        lines.push(format!(
            "- Inclusive/self: {:.1}% / {:.1}%",
            top_func.inclusive_pct, top_func.self_pct
        ));
        lines.push(format!("- Bucket: {}", top_func.bucket.label));

        // Suggested source paths
        if !top_func.bucket.suggested_paths.is_empty() {
            lines.push("- Read first:".to_string());
            for path in &top_func.bucket.suggested_paths {
                lines.push(format!("  - `{}`", path));
            }
        }
    }

    // Profile path
    lines.push(format!("- Profile: `{}`", data.profile_relative_path));

    // Deterministic hint
    let hint = generate_hint(data);
    lines.push(format!("- Hint: {}", hint));

    lines.push(String::new());
}

// ---------------------------------------------------------------------------
//  Enriched per-case summary markdown
// ---------------------------------------------------------------------------

/// Format an enriched per-case `summary.md` combining observations and hotspots.
fn format_enriched_case_summary(data: &CaseSummaryData<'_>, run_paths: &ProfileRunPaths) -> String {
    let obs = data.observation;
    let hotspots = data.hotspots;
    let filter = data.filter;

    let mut lines = Vec::new();

    // Header
    lines.push(format!("# {}", obs.case_name));
    lines.push(String::new());
    lines.push(format!(
        "Command: `{} {}`",
        obs.command,
        obs.command_args.join(" ")
    ));
    lines.push(format!("Group: {}", obs.group_name));
    lines.push(format!("Filter: {}", filter.display_label()));
    lines.push(format!("Wall time: ~{:.0}ms", obs.wall_ms));
    lines.push(format!("Sample count: {}", hotspots.total_sample_count));
    lines.push(format!(
        "Symbolication: {} ({}/{} raw-address hot functions)",
        hotspots.symbolication.status.as_str(),
        hotspots.symbolication.raw_address_function_count,
        hotspots.symbolication.hot_function_count
    ));
    lines.push(String::new());

    // Stage timings
    if !obs.observations.stage_timings.is_empty() {
        lines.push("## Stage timings".to_string());
        lines.push(String::new());
        for metric in &obs.observations.stage_timings {
            lines.push(format!("- {}: ~{:.0}ms", metric.name, metric.value));
        }
        lines.push(String::new());
    }

    // Counters
    if !obs.observations.counters.is_empty() {
        lines.push("## Counters".to_string());
        lines.push(String::new());
        for metric in &obs.observations.counters {
            lines.push(format!("- {}: {}", metric.name, metric.value));
        }
        lines.push(String::new());
    }

    // Hot functions
    if hotspots.symbolication.is_failed() {
        lines.push("## Hot functions".to_string());
        lines.push(String::new());
        lines.push(
            "Function hotspots are raw addresses only; use stage timings and counters from this run, not function names, until symbolication is fixed."
                .to_string(),
        );
        lines.push(format!(
            "Profile shape dump: `cases/{}/profile-shape.txt`",
            obs.case_name
        ));
        lines.push(String::new());
    } else if !hotspots.functions.is_empty() {
        lines.push("## Hot functions".to_string());
        lines.push(String::new());

        for func in &hotspots.functions {
            let display_name = truncate_function_name(&func.name, 72);
            lines.push(format!(
                "- **`{}`** ({}) — {:.1}% incl / {:.1}% self (~{:.0}ms / ~{:.0}ms)",
                display_name,
                func.bucket.label,
                func.inclusive_pct,
                func.self_pct,
                func.estimated_inclusive_ms,
                func.estimated_self_ms,
            ));

            // Show top callers/callees if available (deep mode).
            if !func.top_callers.is_empty() {
                lines.push("  - Callers:".to_string());
                for edge in func.top_callers.iter().take(3) {
                    let caller_name = truncate_function_name(&edge.function_name, 60);
                    lines.push(format!("    - `{}` ({:.1}%)", caller_name, edge.pct));
                }
            }
            if !func.top_callees.is_empty() {
                lines.push("  - Callees:".to_string());
                for edge in func.top_callees.iter().take(3) {
                    let callee_name = truncate_function_name(&edge.function_name, 60);
                    lines.push(format!("    - `{}` ({:.1}%)", callee_name, edge.pct));
                }
            }
        }
        lines.push(String::new());
    }

    // Bucket summary
    let bucket_summary = if hotspots.symbolication.is_failed() {
        Vec::new()
    } else {
        build_bucket_summary(&hotspots.functions)
    };
    if !bucket_summary.is_empty() {
        lines.push("## Buckets".to_string());
        lines.push(String::new());
        for bucket in &bucket_summary {
            lines.push(format!(
                "- **{}** — {} functions, {:.1}% total inclusive",
                bucket.label, bucket.function_count, bucket.total_inclusive_pct
            ));
            for path in &bucket.suggested_paths {
                lines.push(format!("  - `{}`", path));
            }
        }
        lines.push(String::new());
    }

    // Warnings
    if !hotspots.warnings.is_empty() {
        lines.push("## Warnings".to_string());
        lines.push(String::new());
        for warning in &hotspots.warnings {
            lines.push(format!("- {}", warning));
        }
        lines.push(String::new());
    }

    // Hint
    let hint = generate_hint(data);
    lines.push("## Hint".to_string());
    lines.push(String::new());
    lines.push(hint);
    lines.push(String::new());

    // How to open the profile
    lines.push("## Open raw profile".to_string());
    lines.push(String::new());
    lines.push("```bash".to_string());
    lines.push(format!(
        "samply load {}/{}",
        run_paths.root.display(),
        data.profile_relative_path
    ));
    lines.push("```".to_string());
    lines.push(String::new());

    lines.join("\n")
}

// ---------------------------------------------------------------------------
//  Hint generation
// ---------------------------------------------------------------------------

/// Generate a deterministic investigation hint for a case.
///
/// WHAT: Produces a short, actionable hint based on stage timings,
/// hotspot buckets, signal patterns, and symbol availability.
///
/// WHY: Hints guide agents to the right source area without prescribing
/// an optimization strategy. They are deterministic and conservative.
fn generate_hint(data: &CaseSummaryData<'_>) -> String {
    let obs = data.observation;
    let hotspots = data.hotspots;

    // If no hot functions, the profile may be unsymbolicated or unhelpful.
    if hotspots.functions.is_empty() {
        if hotspots.total_sample_count == 0 {
            return "No samples collected; check Samply recording.".to_string();
        }
        return "No hot functions above threshold; try `normal` or `deep` filter for more detail."
            .to_string();
    }

    if hotspots.symbolication.is_failed() {
        return "Function names appear to be raw addresses (not symbolicated); \
                use stage/counter data and retry with symbol dirs or `--presymbolicate` before treating function hotspots as actionable."
            .to_string();
    }

    let top_func = &hotspots.functions[0];

    // Check if alloc dominates self time.
    if top_func.bucket.label == "alloc" && top_func.self_pct > 10.0 {
        return format!(
            "Allocation (`{}`) dominates self time at {:.1}%; \
             check clone/collect/allocation churn in the calling Moth code.",
            truncate_function_name(&top_func.name, 48),
            top_func.self_pct
        );
    }

    // Check if rayon/synchronization dominates.
    if top_func.bucket.label == "rayon" || top_func.name.contains("rayon") {
        return "Rayon/synchronization dominates; check parallel merge/remap \
                and shared-state boundaries in the calling compiler code."
            .to_string();
    }

    // Check for mostly non-Moth functions.
    let moth_count = hotspots
        .functions
        .iter()
        .filter(|f| is_moth_owned(f))
        .count();

    if moth_count == 0 && !hotspots.functions.is_empty() {
        return "Profile is dominated by non-Moth functions; \
                look for the Moth caller edges in the profile to find \
                which compiler code triggers the hot external path."
            .to_string();
    }

    // Find the top stage by timing value.
    let top_stage = obs
        .observations
        .stage_timings
        .iter()
        .max_by(|a, b| a.value.total_cmp(&b.value));

    // Stage-specific hints.
    if let Some(stage) = top_stage {
        // AST + AST bucket: suggest AST owner paths.
        if stage.name == "ast_ms" && top_func.bucket.label == "AST" {
            let paths = format_paths(&top_func.bucket.suggested_paths);
            return format!(
                "AST profile and stage timer agree; inspect repeated type/environment work \
                 before changing HIR.{}",
                paths
            );
        }

        // File prepare + tokenizer/header bucket.
        if stage.name == "file_prepare_ms"
            && (top_func.bucket.label == "Tokenization"
                || top_func.bucket.label == "Header parsing"
                || top_func.bucket.label == "Build system")
        {
            return format!(
                "File preparation dominates; the hot function is in {} ({}). \
                 Inspect tokenization, header parsing, or string-table merge/remap.",
                top_func.bucket.label, top_func.name,
            );
        }

        // HIR + HIR bucket.
        if stage.name == "hir_ms" && top_func.bucket.label == "HIR" {
            let paths = format_paths(&top_func.bucket.suggested_paths);
            return format!(
                "HIR generation and profile agree; inspect HIR lowering logic.{}",
                paths
            );
        }

        // Borrow + borrow bucket.
        if stage.name == "borrow_ms" && top_func.bucket.label == "Borrow validation" {
            let paths = format_paths(&top_func.bucket.suggested_paths);
            return format!(
                "Borrow validation dominates; inspect borrow state representation.{}",
                paths
            );
        }

        // Dependency sort + dependency sorting bucket.
        if stage.name == "dependency_sort_ms" && top_func.bucket.label == "Dependency sorting" {
            return "Dependency sorting dominates; inspect graph traversal and edge handling."
                .to_string();
        }

        // Generic stage hint: bucket paths are still relevant.
        if top_func.bucket.label != "unknown" && top_func.bucket.label != "other" {
            let paths = format_paths(&top_func.bucket.suggested_paths);
            return format!(
                "Top stage is `{}`; hottest function is in {} bucket.{}",
                stage.name, top_func.bucket.label, paths
            );
        }
    }

    // Fallback: top function is Moth-owned but no specific stage pattern matched.
    if is_moth_owned(top_func) {
        let paths = format_paths(&top_func.bucket.suggested_paths);
        return format!(
            "Hottest function is `{}` in {} bucket.{}",
            truncate_function_name(&top_func.name, 48),
            top_func.bucket.label,
            paths
        );
    }

    "Inspect the profile with `samply load` for detailed call stacks.".to_string()
}

/// Check whether a hot function is Moth-owned (not std/alloc/rayon/unknown).
fn is_moth_owned(func: &super::hotspots::ProfileHotFunction) -> bool {
    !matches!(
        func.bucket.label.as_str(),
        "unknown" | "other" | "std" | "core" | "alloc" | "rayon" | "samply/profiler"
    )
}

/// Format suggested source paths into a readable suffix.
fn format_paths(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }

    let path_list: Vec<String> = paths.iter().map(|p| format!("`{}`", p)).collect();
    format!(" See {}", path_list.join(", "))
}

/// Truncate a function name to a maximum length, adding `...` if truncated.
fn truncate_function_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len.saturating_sub(3)])
    }
}

// ---------------------------------------------------------------------------
//  Signal scoring
// ---------------------------------------------------------------------------

/// Compute a combined signal score for ranking cases in the root summary.
///
/// WHAT: Combines normalized wall time, top stage timing, and hottest
/// Moth-owned function inclusive percentage into a single score.
///
/// WHY: A simple weighted sum gives a reasonable ranking that prioritizes
/// cases where profiling is most actionable: slow cases with clear
/// Moth-owned hotspots.
fn combined_signal_score(data: &CaseSummaryData<'_>) -> f64 {
    let wall = data.observation.wall_ms;

    let top_stage_ms = data
        .observation
        .observations
        .stage_timings
        .iter()
        .map(|m| m.value)
        .fold(0.0_f64, f64::max);

    let top_moth_inclusive_pct = data
        .hotspots
        .functions
        .iter()
        .filter(|f| is_moth_owned(f))
        .map(|f| f.inclusive_pct)
        .fold(0.0_f64, f64::max);

    // Weighted sum: wall time and stage timing in milliseconds,
    // inclusive percent weighted to give it reasonable influence.
    wall + top_stage_ms + (top_moth_inclusive_pct * 10.0)
}

// ---------------------------------------------------------------------------
//  Helpers
// ---------------------------------------------------------------------------

/// Round a floating-point value to 2 decimal places.
fn round_2dp(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;
