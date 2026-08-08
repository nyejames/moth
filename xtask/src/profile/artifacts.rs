//! Profile artifact layout and file writing
//!
//! WHAT: Manages the local directory structure for one profiling run and
//! provides helpers to write per-case artifacts (stdout/stderr logs,
//! detailed observations JSON, per-case summary) and root artifacts
//! (run-manifest.json, index.md).
//!
//! WHY: Keeping artifact paths and writers in one module ensures the
//! directory layout stays consistent across all profiling phases and
//! that JSON formatting is centralized rather than scattered across
//! orchestrators and parsers.
//!
//! # What this module owns
//! - `ProfileRunPaths` for the root run directory and manifest
//! - `ProfileCasePaths` for per-case subdirectories
//! - Helpers to write stdout.log, stderr.log, detailed-observations.json,
//!   run-manifest.json, index.md, and per-case hotspots.json
//!
//! # What this module does NOT own
//! - Observation data collection (see `observations.rs`)
//! - Samply runner integration (see `runner.rs`)
//! - Profile JSON parsing or hotspot extraction (see `parse.rs`, `hotspots.rs`)
//! - Agent summaries, enriched per-case summaries, and hint generation
//!   (see `summary.rs`)

use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{BenchmarkMeasurementIdentity, BenchmarkMetric};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use super::hotspots::HotspotExtractionResult;
use super::observations::ProfileObservation;
use super::options::ProfileFilterMode;
use super::parse::ProfileShapeDump;

/// Current on-disk format version for detailed observations.
const OBSERVATIONS_FORMAT_VERSION: u32 = 2;

/// Current on-disk format version for the root run manifest.
const RUN_MANIFEST_FORMAT_VERSION: u32 = 4;

/// Current on-disk format version for per-case hotspot data.
const HOTSPOTS_FORMAT_VERSION: u32 = 1;

/// All paths for one profiling run.
///
/// WHAT: Owns the root run directory path and provides accessors for
/// root-level artifacts.
/// WHY: A single struct prevents path inconsistencies between manifest
/// writing, index generation, and per-case artifact creation.
pub(crate) struct ProfileRunPaths {
    /// Unique run identifier: `<timestamp>-<commit-or-unknown>`.
    pub(crate) run_id: String,
    /// Root directory for this run: `benchmarks/local-data/profiles/<run-id>/`.
    pub(crate) root: PathBuf,
}

/// All paths for one case within a profiling run.
///
/// WHAT: Owns the per-case subdirectory and provides accessors for
/// stdout, stderr, observations, and summary artifacts.
/// WHY: Per-case path construction is repeated for every case; a struct
/// keeps the layout deterministic and testable.
pub(crate) struct ProfileCasePaths {
    /// Case subdirectory: `<run-root>/cases/<case-id>/`.
    pub(crate) case_dir: PathBuf,
    /// Path to stdout.log within the case directory.
    pub(crate) stdout_log: PathBuf,
    /// Path to stderr.log within the case directory.
    pub(crate) stderr_log: PathBuf,
    /// Path to detailed-observations.json within the case directory.
    pub(crate) observations_json: PathBuf,
    /// Path to summary.md within the case directory.
    pub(crate) summary_md: PathBuf,
    /// Path to profile.json.gz (Samply output) within the case directory.
    ///
    /// Samply 0.13.1 writes gzip-compressed profiles regardless of the
    /// requested extension. We use `.json.gz` to match the actual format.
    pub(crate) profile_json: PathBuf,
    /// Path to hotspots.json within the case directory.
    ///
    /// Written after parsing the Samply profile. Contains ranked function
    /// hotspots with percentage and millisecond estimates.
    pub(crate) hotspots_json: PathBuf,
    /// Path to profile-shape.txt within the case directory.
    ///
    /// Written only when symbolication fails so the next investigation can
    /// inspect the profile's structural tables without opening the raw JSON.
    pub(crate) profile_shape_txt: PathBuf,
}

/// Manifest data for one case, written into the root run-manifest.json.
///
/// WHAT: Compact representation of one case's artifacts and timing.
/// WHY: The manifest is a machine-readable index of the run so later
/// phases can discover cases without scanning directories.
pub(crate) struct ProfileCaseManifest {
    pub(crate) case_id: String,
    pub(crate) identity: BenchmarkMeasurementIdentity,
    pub(crate) group_name: String,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) observation_wall_ms: f64,
    pub(crate) profile_path: String,
    pub(crate) stdout_path: String,
    pub(crate) stderr_path: String,
    /// Relative summary path, absent for filters such as `raw-index` that do not write summaries.
    pub(crate) summary_path: Option<String>,
}

impl ProfileRunPaths {
    /// Create a new run directory with a unique run id.
    ///
    /// The run id is `<YYYY-MM-DDThh-mm>-<short-commit-or-unknown>`.
    /// Creates the root directory and `cases/` subdirectory.
    pub(crate) fn create(profiles_root: &Path, commit: Option<&str>) -> Result<Self, String> {
        let ts = BenchmarkTimestamp::now();
        let commit_label = commit.unwrap_or("unknown");

        // Include seconds in the run id so back-to-back runs do not collide
        // when the same commit is profiled more than once in a minute.
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() % 60)
            .unwrap_or(0);

        let run_id = format!(
            "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}-{}",
            ts.year, ts.month, ts.day, ts.hour, ts.minute, seconds, commit_label
        );

        let root = profiles_root.join(&run_id);

        fs::create_dir_all(root.join("cases")).map_err(|e| {
            format!(
                "Failed to create profile run directory '{}': {}",
                root.display(),
                e
            )
        })?;

        Ok(Self { run_id, root })
    }

    /// Return the path for the root run-manifest.json.
    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.root.join("run-manifest.json")
    }

    /// Return the path for the root index.md.
    pub(crate) fn index_path(&self) -> PathBuf {
        self.root.join("index.md")
    }

    /// Build case paths for an authored case ID.
    pub(crate) fn case_paths(&self, case_id: &str) -> ProfileCasePaths {
        let case_dir = self.root.join("cases").join(case_id);
        ProfileCasePaths {
            case_dir: case_dir.clone(),
            stdout_log: case_dir.join("stdout.log"),
            stderr_log: case_dir.join("stderr.log"),
            observations_json: case_dir.join("detailed-observations.json"),
            summary_md: case_dir.join("summary.md"),
            profile_json: case_dir.join("profile.json.gz"),
            hotspots_json: case_dir.join("hotspots.json"),
            profile_shape_txt: case_dir.join("profile-shape.txt"),
        }
    }
}

impl ProfileCasePaths {
    /// Create the case subdirectory on disk.
    pub(crate) fn create_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.case_dir).map_err(|e| {
            format!(
                "Failed to create case directory '{}': {}",
                self.case_dir.display(),
                e
            )
        })
    }

    /// Write stdout content to stdout.log.
    pub(crate) fn write_stdout(&self, content: &str) -> Result<(), String> {
        fs::write(&self.stdout_log, content).map_err(|e| {
            format!(
                "Failed to write stdout.log '{}': {}",
                self.stdout_log.display(),
                e
            )
        })
    }

    /// Write stderr content to stderr.log.
    pub(crate) fn write_stderr(&self, content: &str) -> Result<(), String> {
        fs::write(&self.stderr_log, content).map_err(|e| {
            format!(
                "Failed to write stderr.log '{}': {}",
                self.stderr_log.display(),
                e
            )
        })
    }

    /// Write detailed-observations.json from a profile observation.
    pub(crate) fn write_observations_json(
        &self,
        observation: &ProfileObservation,
    ) -> Result<(), String> {
        validate_observation_finite(observation)?;
        let json = serde_json::to_string_pretty(&DetailedObservationsFile::from(observation))
            .map_err(|error| format!("Failed to serialize detailed observations: {error}"))?;
        fs::write(&self.observations_json, json).map_err(|e| {
            format!(
                "Failed to write detailed-observations.json '{}': {}",
                self.observations_json.display(),
                e
            )
        })
    }
}

/// Write per-case hotspots.json from a hotspot extraction result.
///
/// WHAT: Serializes the hotspot extraction output as compact JSON and
/// writes it to the case directory.
///
/// WHY: `hotspots.json` is the machine-readable per-case hotspot data
/// that agents read to identify optimization targets. Using serde_json
/// ensures correct JSON escaping and formatting.
pub(crate) fn write_hotspots_json(
    case_paths: &ProfileCasePaths,
    result: &HotspotExtractionResult,
) -> Result<(), String> {
    validate_hotspots_finite(result)?;
    let json = serde_json::to_string_pretty(&HotspotsFile::from(result))
        .map_err(|error| format!("Failed to serialize hotspots: {error}"))?;
    fs::write(&case_paths.hotspots_json, json).map_err(|e| {
        format!(
            "Failed to write hotspots.json '{}': {}",
            case_paths.hotspots_json.display(),
            e
        )
    })
}

pub(crate) fn write_profile_shape_dump(
    case_paths: &ProfileCasePaths,
    shape: &ProfileShapeDump,
) -> Result<(), String> {
    let text = format_profile_shape_dump(shape);
    fs::write(&case_paths.profile_shape_txt, text).map_err(|e| {
        format!(
            "Failed to write profile-shape.txt '{}': {}",
            case_paths.profile_shape_txt.display(),
            e
        )
    })
}

/// Write the root run-manifest.json containing all case manifests.
pub(crate) fn write_run_manifest(
    run_paths: &ProfileRunPaths,
    run_id: &str,
    git_revision: Option<&crate::bench_types::GitRevision>,
    filter: ProfileFilterMode,
    samply_rate_hz: Option<f64>,
    cases: &[ProfileCaseManifest],
) -> Result<(), String> {
    validate_manifest_finite(cases)?;
    let manifest = RunManifestFile::new(run_id, git_revision, filter, samply_rate_hz, cases);
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("Failed to serialize run-manifest.json: {error}"))?;
    fs::write(run_paths.manifest_path(), json).map_err(|e| {
        format!(
            "Failed to write run-manifest.json '{}': {}",
            run_paths.manifest_path().display(),
            e
        )
    })
}

/// Write the root index.md summarizing the profiling run.
pub(crate) fn write_index_md(
    run_paths: &ProfileRunPaths,
    run_id: &str,
    filter: ProfileFilterMode,
    cases: &[ProfileCaseManifest],
) -> Result<(), String> {
    let md = format_index_md(run_id, filter, cases);
    fs::write(run_paths.index_path(), md).map_err(|e| {
        format!(
            "Failed to write index.md '{}': {}",
            run_paths.index_path().display(),
            e
        )
    })
}

// ---------------------------------------------------------------------------
//  JSON formatting
// ---------------------------------------------------------------------------

fn format_profile_shape_dump(shape: &ProfileShapeDump) -> String {
    let mut lines = Vec::new();

    lines.push(format!("meta.product: {}", shape.meta_product));
    lines.push(format!("meta.version: {}", shape.meta_version));
    lines.push(format!("threads: {}", shape.thread_count));
    lines.push(format!(
        "first thread funcTable keys: {}",
        format_inline_list(&shape.first_thread_func_table_keys)
    ));
    lines.push("first 20 func names:".to_string());
    append_bullets(&mut lines, &shape.first_20_func_names);
    lines.push(format!(
        "resourceTable keys: {}",
        format_inline_list(&shape.resource_table_keys)
    ));
    lines.push(format!(
        "libs count: {}",
        shape
            .libs_count
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    ));
    lines.push("first 10 libs:".to_string());
    append_bullets(&mut lines, &shape.first_10_libs);
    lines.push(format!(
        "nativeSymbols present: {}",
        if shape.native_symbols_present {
            "yes"
        } else {
            "no"
        }
    ));

    lines.join("\n")
}

fn format_inline_list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

fn append_bullets(lines: &mut Vec<String>, items: &[String]) {
    if items.is_empty() {
        lines.push("  - none".to_string());
        return;
    }

    for item in items {
        lines.push(format!("  - {item}"));
    }
}

fn validate_observation_finite(observation: &ProfileObservation) -> Result<(), String> {
    super::observations::require_finite(observation.wall_ms, "wall_ms")?;
    for metric in observation
        .observations
        .stage_timings
        .iter()
        .chain(&observation.observations.counters)
    {
        super::observations::require_finite(metric.value, "metric value")?;
    }
    Ok(())
}

fn validate_manifest_finite(cases: &[ProfileCaseManifest]) -> Result<(), String> {
    for case in cases {
        super::observations::require_finite(case.observation_wall_ms, "observation_wall_ms")?;
    }
    Ok(())
}

fn validate_hotspots_finite(result: &HotspotExtractionResult) -> Result<(), String> {
    super::observations::require_finite(result.total_sample_weight, "total_sample_weight")?;
    super::observations::require_finite(result.wall_time_ms, "wall_time_ms")?;
    for function in &result.functions {
        super::observations::require_finite(function.inclusive_samples, "inclusive_samples")?;
        super::observations::require_finite(function.self_samples, "self_samples")?;
        super::observations::require_finite(function.inclusive_pct, "inclusive_pct")?;
        super::observations::require_finite(function.self_pct, "self_pct")?;
        super::observations::require_finite(
            function.estimated_inclusive_ms,
            "estimated_inclusive_ms",
        )?;
        super::observations::require_finite(function.estimated_self_ms, "estimated_self_ms")?;
    }
    Ok(())
}

/// Serde-backed shape of detailed-observations.json.
///
/// Every profile-owned writer returns `Result`; there is no manual string
/// assembly and no fallback payload.
#[derive(Serialize)]
struct DetailedObservationsFile {
    format_version: u32,
    case_id: String,
    group: String,
    command: Vec<String>,
    wall_ms: f64,
    stage_timings: Vec<BenchmarkMetric>,
    counters: Vec<BenchmarkMetric>,
}

impl From<&ProfileObservation> for DetailedObservationsFile {
    fn from(observation: &ProfileObservation) -> Self {
        let mut command = Vec::with_capacity(observation.command_args.len() + 1);
        command.push(observation.command.clone());
        command.extend(observation.command_args.iter().cloned());

        Self {
            format_version: OBSERVATIONS_FORMAT_VERSION,
            case_id: observation.case_id.clone(),
            group: observation.group_name.clone(),
            command,
            wall_ms: observation.wall_ms,
            stage_timings: observation.observations.stage_timings.clone(),
            counters: observation.observations.counters.clone(),
        }
    }
}

/// Serde-backed shape of the root run-manifest.json.
#[derive(Serialize)]
struct RunManifestFile {
    format_version: u32,
    run_id: String,
    timestamp: String,
    commit: Option<String>,
    git_dirty: Option<bool>,
    filter: String,
    samply_rate_hz: Option<f64>,
    cases: Vec<RunManifestCase>,
}

/// One case entry in the root run-manifest.json.
#[derive(Serialize)]
struct RunManifestCase {
    case_id: String,
    group_name: String,
    workload_id: String,
    source_fingerprint: String,
    measurement_fingerprint: String,
    timing_schema_version: u32,
    command: String,
    args: Vec<String>,
    observation_wall_ms: f64,
    profile_path: String,
    stdout_path: String,
    stderr_path: String,
    summary_path: Option<String>,
}

impl RunManifestFile {
    fn new(
        run_id: &str,
        git_revision: Option<&crate::bench_types::GitRevision>,
        filter: ProfileFilterMode,
        samply_rate_hz: Option<f64>,
        cases: &[ProfileCaseManifest],
    ) -> Self {
        Self {
            format_version: RUN_MANIFEST_FORMAT_VERSION,
            run_id: run_id.to_string(),
            timestamp: BenchmarkTimestamp::now().format_run_header(),
            commit: git_revision.and_then(|revision| revision.commit.clone()),
            git_dirty: git_revision.and_then(|revision| revision.dirty),
            filter: filter.display_label().to_string(),
            samply_rate_hz,
            cases: cases.iter().map(RunManifestCase::from).collect(),
        }
    }
}

impl From<&ProfileCaseManifest> for RunManifestCase {
    fn from(case: &ProfileCaseManifest) -> Self {
        Self {
            case_id: case.case_id.clone(),
            group_name: case.group_name.clone(),
            workload_id: case.identity.workload_id.clone(),
            source_fingerprint: case.identity.source_fingerprint.clone(),
            measurement_fingerprint: case.identity.measurement_fingerprint.clone(),
            timing_schema_version: case.identity.timing_schema_version,
            command: case.command.clone(),
            args: case.args.clone(),
            observation_wall_ms: case.observation_wall_ms,
            profile_path: case.profile_path.clone(),
            stdout_path: case.stdout_path.clone(),
            stderr_path: case.stderr_path.clone(),
            summary_path: case.summary_path.clone(),
        }
    }
}

/// Serde-backed shape of per-case hotspots.json.
#[derive(Serialize)]
struct HotspotsFile {
    format_version: u32,
    total_sample_count: usize,
    total_sample_weight: f64,
    wall_time_ms: f64,
    hot_function_count: usize,
    symbolication: SymbolicationFile,
    functions: Vec<HotspotFunctionFile>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct SymbolicationFile {
    status: String,
    raw_address_function_count: usize,
    hot_function_count: usize,
    raw_address_ratio: f64,
}

#[derive(Serialize)]
struct HotspotFunctionFile {
    name: String,
    bucket: HotspotBucketFile,
    inclusive_samples: f64,
    self_samples: f64,
    inclusive_pct: f64,
    self_pct: f64,
    estimated_inclusive_ms: f64,
    estimated_self_ms: f64,
    top_callers: Vec<HotspotEdgeFile>,
    top_callees: Vec<HotspotEdgeFile>,
}

#[derive(Serialize)]
struct HotspotBucketFile {
    label: String,
    suggested_paths: Vec<String>,
}

#[derive(Serialize)]
struct HotspotEdgeFile {
    function_name: String,
    samples: f64,
    pct: f64,
}

impl From<&HotspotExtractionResult> for HotspotsFile {
    fn from(result: &HotspotExtractionResult) -> Self {
        Self {
            format_version: HOTSPOTS_FORMAT_VERSION,
            total_sample_count: result.total_sample_count,
            total_sample_weight: round_2dp(result.total_sample_weight),
            wall_time_ms: round_2dp(result.wall_time_ms),
            hot_function_count: result.functions.len(),
            symbolication: SymbolicationFile {
                status: result.symbolication.status.as_str().to_string(),
                raw_address_function_count: result.symbolication.raw_address_function_count,
                hot_function_count: result.symbolication.hot_function_count,
                raw_address_ratio: round_2dp(result.symbolication.raw_address_ratio),
            },
            functions: result
                .functions
                .iter()
                .map(|func| HotspotFunctionFile {
                    name: func.name.clone(),
                    bucket: HotspotBucketFile {
                        label: func.bucket.label.clone(),
                        suggested_paths: func.bucket.suggested_paths.clone(),
                    },
                    inclusive_samples: func.inclusive_samples,
                    self_samples: func.self_samples,
                    inclusive_pct: round_2dp(func.inclusive_pct),
                    self_pct: round_2dp(func.self_pct),
                    estimated_inclusive_ms: round_2dp(func.estimated_inclusive_ms),
                    estimated_self_ms: round_2dp(func.estimated_self_ms),
                    top_callers: func.top_callers.iter().map(HotspotEdgeFile::from).collect(),
                    top_callees: func.top_callees.iter().map(HotspotEdgeFile::from).collect(),
                })
                .collect(),
            warnings: result.warnings.clone(),
        }
    }
}

impl From<&super::parse::ProfileEdge> for HotspotEdgeFile {
    fn from(edge: &super::parse::ProfileEdge) -> Self {
        Self {
            function_name: edge.function_name.clone(),
            samples: round_2dp(edge.samples),
            pct: round_2dp(edge.pct),
        }
    }
}

/// Round a floating-point value to 2 decimal places.
fn round_2dp(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

// ---------------------------------------------------------------------------
//  Markdown formatting
// ---------------------------------------------------------------------------

/// Format the root index.md for a profiling run.
fn format_index_md(
    run_id: &str,
    filter: ProfileFilterMode,
    cases: &[ProfileCaseManifest],
) -> String {
    let mut lines = Vec::new();

    lines.push(format!("# Profiling run: {}", run_id));
    lines.push(String::new());
    lines.push(format!("Filter: {}", filter.display_label()));
    lines.push(format!("Cases: {}", cases.len()));
    lines.push(String::new());

    if !cases.is_empty() {
        lines.push("## Cases".to_string());
        lines.push(String::new());
        for case in cases {
            lines.push(format!(
                "- **{}** — `{}` (~{:.0}ms)",
                case.case_id,
                format_command(&case.command, &case.args),
                case.observation_wall_ms,
            ));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Format a command with args for display.
fn format_command(command: &str, args: &[String]) -> String {
    let mut parts = vec![command.to_string()];
    parts.extend(args.iter().cloned());
    parts.join(" ")
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "artifacts_tests.rs"]
mod tests;
