//! Typed authority for the repository benchmark inventory.
//!
//! WHAT: Loads the schema-4 TOML manifest, validates authored identities,
//! resolves each case to one immutable workload relationship and resolves each
//! scaling series to its ordered member cases.
//! WHY: Benchmark commands need one strict source of case order, runner
//! semantics and filesystem ownership instead of path-derived text lists.

use crate::bench_types::BenchmarkGroup;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub(crate) const BENCHMARK_MANIFEST_PATH: &str = "benchmarks/manifest.toml";
pub(crate) const BENCHMARK_MANIFEST_SCHEMA_VERSION: u32 = 4;

/// Authored fingerprint boundary mode for one workload.
///
/// `full_tree` means the complete entry file or directory forms the authored
/// source boundary, minus explicit generated-output excludes. `partitioned`
/// means an author deliberately lists disjoint roots under one directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BenchmarkFingerprintMode {
    FullTree,
    Partitioned,
}

/// A fully validated benchmark inventory in manifest order.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkManifest {
    pub(crate) workloads: Vec<BenchmarkWorkload>,
    pub(crate) cases: Vec<BenchmarkCase>,
    pub(crate) scaling_series: Vec<BenchmarkScalingSeries>,
    pub(crate) manifest_path: PathBuf,
    pub(crate) repository_root: PathBuf,
}

/// One declared scaling series: the same compiler work at several input sizes.
///
/// WHY: the normal suites compare a case against its own recorded history, so a
/// cost that has been superlinear since it was written never reads as a
/// regression. A series states the input size explicitly, which lets the growth
/// exponent be fitted and held to a budget.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BenchmarkScalingSeries {
    pub(crate) id: String,
    /// Stable timing metric name fitted against size. Must be a Basic metric:
    /// the benchmark compiler is built with `timers`, not `detailed_timers`.
    pub(crate) metric: String,
    /// Largest growth exponent this series is allowed to reach.
    pub(crate) max_exponent: f64,
    /// Member points in strictly increasing size order.
    pub(crate) points: Vec<BenchmarkScalingPoint>,
}

/// One member of a scaling series, resolved to its manifest case.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BenchmarkScalingPoint {
    pub(crate) case_id: String,
    pub(crate) case_index: usize,
    /// Authored input size this fixture represents.
    pub(crate) size: u32,
}

/// Whether a workload entry is a single file or a directory project.
///
/// Discovered once during manifest validation so later execution phases
/// never need to probe the filesystem to decide working-directory policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkEntryKind {
    File,
    Directory,
}

/// One repository-relative benchmark input boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BenchmarkWorkload {
    pub(crate) id: String,
    pub(crate) entry: PathBuf,
    pub(crate) entry_kind: BenchmarkEntryKind,
    pub(crate) fingerprint_mode: BenchmarkFingerprintMode,
    pub(crate) fingerprint_roots: Vec<PathBuf>,
    pub(crate) fingerprint_excludes: Vec<PathBuf>,
    /// Declared generated output roots, relative to the workload entry.
    ///
    /// These are cleanup authority only: they are validated as absent before
    /// the first `build` execution and removed by the run workspace on
    /// finalisation. They never act as source or config semantics.
    pub(crate) generated_output_roots: Vec<PathBuf>,
}

/// One authored case with a compact index into the manifest workloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BenchmarkCase {
    pub(crate) id: String,
    pub(crate) case_index: usize,
    pub(crate) workload_index: usize,
    pub(crate) group_name: BenchmarkGroup,
    pub(crate) quick: bool,
    pub(crate) expectation: BenchmarkExpectation,
    pub(crate) runner: BenchmarkRunner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BenchmarkRunner {
    Cli {
        command: CliBenchmarkCommand,
        args: Vec<String>,
    },
    Frontend {
        profile: FrontendBenchmarkProfile,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CliBenchmarkCommand {
    Check,
    Build,
}

impl CliBenchmarkCommand {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FrontendBenchmarkProfile {
    Dev,
}

impl FrontendBenchmarkProfile {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkExpectation {
    Clean,
}

/// Resolved CLI arguments for one manifest case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CliBenchmarkInvocation {
    pub(crate) command: CliBenchmarkCommand,
    pub(crate) args: Vec<String>,
    pub(crate) current_directory: PathBuf,
}

/// Resolved frontend input for one manifest case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrontendBenchmarkInvocation {
    pub(crate) entry: PathBuf,
    pub(crate) profile: FrontendBenchmarkProfile,
}

#[derive(Debug)]
pub(crate) enum BenchmarkManifestError {
    Read {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    Invalid {
        path: PathBuf,
        subject: String,
        message: String,
    },
    WorkloadPath {
        manifest_path: PathBuf,
        workload_id: String,
        field: &'static str,
        authored_path: String,
        source: io::Error,
    },
}

impl Display for BenchmarkManifestError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "failed to read manifest '{}': {source}",
                    path.display()
                )
            }
            Self::Parse { path, source } => {
                write!(
                    formatter,
                    "failed to parse manifest '{}': {source}",
                    path.display()
                )
            }
            Self::Invalid {
                path,
                subject,
                message,
            } => {
                write!(
                    formatter,
                    "invalid manifest '{}' for {}: {message}",
                    path.display(),
                    subject
                )
            }
            Self::WorkloadPath {
                manifest_path,
                workload_id,
                field,
                authored_path,
                source,
            } => {
                write!(
                    formatter,
                    "failed to resolve {field} '{authored_path}' for workload '{workload_id}' in manifest '{}': {source}",
                    manifest_path.display()
                )
            }
        }
    }
}

impl std::error::Error for BenchmarkManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::WorkloadPath { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}

#[derive(Debug)]
struct ValidatedWorkloadPath {
    relative: PathBuf,
    canonical: PathBuf,
    is_directory: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBenchmarkManifest {
    schema: u32,
    #[serde(rename = "workload")]
    workloads: Vec<RawBenchmarkWorkload>,
    #[serde(rename = "case")]
    cases: Vec<RawBenchmarkCase>,
    #[serde(rename = "scaling", default)]
    scaling_series: Vec<RawBenchmarkScalingSeries>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBenchmarkScalingSeries {
    id: String,
    metric: String,
    max_exponent: f64,
    points: Vec<RawBenchmarkScalingPoint>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBenchmarkScalingPoint {
    case: String,
    size: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBenchmarkWorkload {
    id: String,
    entry: String,
    fingerprint_mode: BenchmarkFingerprintMode,
    fingerprint_roots: Vec<String>,
    fingerprint_excludes: Vec<String>,
    #[serde(default)]
    generated_output_roots: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBenchmarkCase {
    id: String,
    workload: String,
    group: String,
    quick: bool,
    expectation: String,
    runner: RawBenchmarkRunner,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum RawBenchmarkRunner {
    #[serde(rename = "cli")]
    Cli { command: String, args: Vec<String> },
    #[serde(rename = "frontend")]
    Frontend { profile: String },
}

/// Load the repository's one benchmark manifest.
pub(crate) fn load_benchmark_manifest() -> Result<BenchmarkManifest, BenchmarkManifestError> {
    let current_directory =
        std::env::current_dir().map_err(|source| BenchmarkManifestError::Read {
            path: PathBuf::from("."),
            source,
        })?;

    load_benchmark_manifest_from(&current_directory)
}

pub(crate) fn load_benchmark_manifest_from(
    current_directory: &Path,
) -> Result<BenchmarkManifest, BenchmarkManifestError> {
    let repository_root =
        find_repository_root(current_directory).ok_or_else(|| BenchmarkManifestError::Read {
            path: current_directory.join(BENCHMARK_MANIFEST_PATH),
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "could not locate the repository benchmark manifest",
            ),
        })?;
    let manifest_path = repository_root.join(BENCHMARK_MANIFEST_PATH);
    load_manifest_at(&manifest_path, &repository_root)
}

/// Load a manifest against an explicit repository root for focused tests.
pub(crate) fn load_manifest_at(
    manifest_path: &Path,
    repository_root: &Path,
) -> Result<BenchmarkManifest, BenchmarkManifestError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|source| BenchmarkManifestError::Read {
            path: manifest_path.to_owned(),
            source,
        })?;
    let raw: RawBenchmarkManifest =
        toml::from_str(&contents).map_err(|source| BenchmarkManifestError::Parse {
            path: manifest_path.to_owned(),
            source,
        })?;
    let canonical_repository_root =
        fs::canonicalize(repository_root).map_err(|source| BenchmarkManifestError::Read {
            path: repository_root.to_owned(),
            source,
        })?;

    validate_manifest(raw, manifest_path, &canonical_repository_root)
}

impl BenchmarkManifest {
    pub(crate) fn workload_for(&self, case: &BenchmarkCase) -> Option<&BenchmarkWorkload> {
        self.workloads.get(case.workload_index)
    }

    pub(crate) fn cli_cases(&self) -> impl Iterator<Item = &BenchmarkCase> {
        self.cases
            .iter()
            .filter(|case| matches!(case.runner, BenchmarkRunner::Cli { .. }))
    }

    pub(crate) fn frontend_cases(&self) -> impl Iterator<Item = &BenchmarkCase> {
        self.cases
            .iter()
            .filter(|case| matches!(case.runner, BenchmarkRunner::Frontend { .. }))
    }

    pub(crate) fn frontend_invocation(
        &self,
        case: &BenchmarkCase,
    ) -> Result<FrontendBenchmarkInvocation, BenchmarkManifestError> {
        let workload = self
            .workload_for(case)
            .ok_or_else(|| self.runtime_error(case, "workload relationship is invalid"))?;
        let BenchmarkRunner::Frontend { profile } = case.runner else {
            return Err(self.runtime_error(case, "case does not declare a frontend runner"));
        };

        Ok(FrontendBenchmarkInvocation {
            entry: self.repository_root.join(&workload.entry),
            profile,
        })
    }

    pub(crate) fn runtime_error(
        &self,
        case: &BenchmarkCase,
        message: &str,
    ) -> BenchmarkManifestError {
        BenchmarkManifestError::Invalid {
            path: self.manifest_path.clone(),
            subject: format!("case '{}'", case.id),
            message: message.to_owned(),
        }
    }
}

fn validate_manifest(
    raw: RawBenchmarkManifest,
    manifest_path: &Path,
    repository_root: &Path,
) -> Result<BenchmarkManifest, BenchmarkManifestError> {
    if raw.schema != BENCHMARK_MANIFEST_SCHEMA_VERSION {
        return Err(invalid(
            manifest_path,
            "manifest",
            format!(
                "schema must equal {BENCHMARK_MANIFEST_SCHEMA_VERSION}, got {}",
                raw.schema
            ),
        ));
    }
    if raw.workloads.is_empty() {
        return Err(invalid(
            manifest_path,
            "manifest",
            "workload inventory must not be empty",
        ));
    }
    if raw.cases.is_empty() {
        return Err(invalid(
            manifest_path,
            "manifest",
            "case inventory must not be empty",
        ));
    }

    let mut workload_ids = HashMap::new();
    let mut canonical_entry_owners = HashMap::new();
    let mut all_ids = HashSet::new();
    let mut workloads = Vec::with_capacity(raw.workloads.len());
    for raw_workload in raw.workloads {
        validate_id(manifest_path, "workload", &raw_workload.id)?;
        if !all_ids.insert(raw_workload.id.clone()) {
            return Err(invalid(
                manifest_path,
                format!("workload '{}'", raw_workload.id),
                "duplicate global ID",
            ));
        }
        workload_ids.insert(raw_workload.id.clone(), workloads.len());

        let entry = validate_existing_workload_path(
            manifest_path,
            repository_root,
            &raw_workload.id,
            "entry",
            &raw_workload.entry,
        )?;
        if let Some(existing_workload_id) = canonical_entry_owners.get(&entry.canonical) {
            return Err(invalid(
                manifest_path,
                format!("workload '{}'", raw_workload.id),
                format!(
                    "entry '{}' resolves to the same repository path as workload '{}'",
                    raw_workload.entry, existing_workload_id
                ),
            ));
        }
        canonical_entry_owners.insert(entry.canonical.clone(), raw_workload.id.clone());

        let mut fingerprint_roots = Vec::with_capacity(raw_workload.fingerprint_roots.len());
        for root in raw_workload.fingerprint_roots {
            let root_path = validate_existing_workload_path(
                manifest_path,
                repository_root,
                &raw_workload.id,
                "fingerprint root",
                &root,
            )?;
            fingerprint_roots.push(root_path);
        }

        // Validate fingerprint roots against the declared boundary mode.
        validate_fingerprint_mode(
            manifest_path,
            &raw_workload.id,
            raw_workload.fingerprint_mode,
            &entry,
            &fingerprint_roots,
        )?;

        if fingerprint_roots.is_empty() {
            return Err(invalid(
                manifest_path,
                format!("workload '{}'", raw_workload.id),
                "fingerprint_roots must not be empty",
            ));
        }

        let mut fingerprint_excludes = Vec::with_capacity(raw_workload.fingerprint_excludes.len());
        for exclude in raw_workload.fingerprint_excludes {
            let exclude_path = validate_fingerprint_exclude(
                manifest_path,
                repository_root,
                &raw_workload.id,
                &exclude,
                &fingerprint_roots,
                &fingerprint_excludes,
            )?;
            fingerprint_excludes.push(exclude_path);
        }

        let generated_output_roots = validate_generated_output_roots(
            manifest_path,
            &raw_workload.id,
            &entry,
            &raw_workload.generated_output_roots,
            &fingerprint_excludes,
        )?;

        workloads.push(BenchmarkWorkload {
            id: raw_workload.id,
            entry: entry.relative,
            entry_kind: if entry.is_directory {
                BenchmarkEntryKind::Directory
            } else {
                BenchmarkEntryKind::File
            },
            fingerprint_mode: raw_workload.fingerprint_mode,
            fingerprint_roots: fingerprint_roots
                .into_iter()
                .map(|root| root.relative)
                .collect(),
            fingerprint_excludes,
            generated_output_roots,
        });
    }

    let mut used_workloads = vec![false; workloads.len()];
    let mut invocation_keys = HashSet::new();
    let mut cases = Vec::with_capacity(raw.cases.len());

    for raw_case in raw.cases {
        validate_id(manifest_path, "case", &raw_case.id)?;
        if !all_ids.insert(raw_case.id.clone()) {
            return Err(invalid(
                manifest_path,
                format!("case '{}'", raw_case.id),
                "duplicate global ID",
            ));
        }
        let group_name = BenchmarkGroup::parse_spelling(&raw_case.group).ok_or_else(|| {
            invalid(
                manifest_path,
                format!("case '{}'", raw_case.id),
                format!("unknown group '{}'", raw_case.group),
            )
        })?;
        let expectation = match raw_case.expectation.as_str() {
            "clean" => BenchmarkExpectation::Clean,
            other => {
                return Err(invalid(
                    manifest_path,
                    format!("case '{}'", raw_case.id),
                    format!("unknown expectation '{other}'"),
                ));
            }
        };
        let Some(&workload_index) = workload_ids.get(&raw_case.workload) else {
            return Err(invalid(
                manifest_path,
                format!("case '{}'", raw_case.id),
                format!("unknown workload '{}'", raw_case.workload),
            ));
        };
        used_workloads[workload_index] = true;

        let runner = match raw_case.runner {
            RawBenchmarkRunner::Cli { command, args } => {
                let command = match command.as_str() {
                    "check" => CliBenchmarkCommand::Check,
                    "build" => CliBenchmarkCommand::Build,
                    other => {
                        return Err(invalid(
                            manifest_path,
                            format!("case '{}'", raw_case.id),
                            format!("unknown CLI command '{other}'"),
                        ));
                    }
                };
                let key = InvocationKey::Cli {
                    workload_index,
                    command,
                    args: args.clone(),
                };
                if !invocation_keys.insert(key) {
                    return Err(invalid(
                        manifest_path,
                        format!("case '{}'", raw_case.id),
                        "duplicate workload and complete CLI runner invocation",
                    ));
                }
                BenchmarkRunner::Cli { command, args }
            }
            RawBenchmarkRunner::Frontend { profile } => {
                let profile = match profile.as_str() {
                    "dev" => FrontendBenchmarkProfile::Dev,
                    other => {
                        return Err(invalid(
                            manifest_path,
                            format!("case '{}'", raw_case.id),
                            format!("unknown frontend profile '{other}'"),
                        ));
                    }
                };
                let key = InvocationKey::Frontend {
                    workload_index,
                    profile,
                };
                if !invocation_keys.insert(key) {
                    return Err(invalid(
                        manifest_path,
                        format!("case '{}'", raw_case.id),
                        "duplicate workload and complete frontend runner invocation",
                    ));
                }
                BenchmarkRunner::Frontend { profile }
            }
        };

        if let BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Build,
            ..
        } = runner
        {
            let workload = &workloads[workload_index];
            if workload.entry_kind == BenchmarkEntryKind::Directory
                && workload.generated_output_roots.is_empty()
            {
                return Err(invalid(
                    manifest_path,
                    format!("case '{}'", raw_case.id),
                    "directory workload with a CLI build case must declare at least one generated output root",
                ));
            }
        }

        cases.push(BenchmarkCase {
            id: raw_case.id,
            case_index: cases.len(),
            workload_index,
            group_name,
            quick: raw_case.quick,
            expectation,
            runner,
        });
    }

    if let Some((index, false)) = used_workloads.iter().enumerate().find(|(_, used)| !**used) {
        return Err(invalid(
            manifest_path,
            format!("workload '{}'", workloads[index].id),
            "workload is not referenced by any case",
        ));
    }

    let scaling_series =
        validate_scaling_series(raw.scaling_series, &cases, &mut all_ids, manifest_path)?;

    Ok(BenchmarkManifest {
        workloads,
        cases,
        scaling_series,
        manifest_path: manifest_path.to_owned(),
        repository_root: repository_root.to_owned(),
    })
}

/// Minimum member count for a fitted scaling series.
///
/// WHY: two points give a single ratio, which one noisy run can dominate. Three
/// points make the fit a trend rather than a comparison.
const MINIMUM_SCALING_POINTS: usize = 3;

/// Resolve every declared scaling series against the validated case inventory.
///
/// WHAT: checks series identity, member existence, runner agreement, strictly
/// increasing sizes and a usable exponent budget.
/// WHY: the fitted exponent is only meaningful when every point runs the same
/// runner over the same shape and differs only in the declared size. An
/// unchecked series would report a confident number about nothing.
fn validate_scaling_series(
    raw_series: Vec<RawBenchmarkScalingSeries>,
    cases: &[BenchmarkCase],
    all_ids: &mut HashSet<String>,
    manifest_path: &Path,
) -> Result<Vec<BenchmarkScalingSeries>, BenchmarkManifestError> {
    let case_indexes: HashMap<&str, usize> = cases
        .iter()
        .enumerate()
        .map(|(index, case)| (case.id.as_str(), index))
        .collect();

    let mut series_list = Vec::with_capacity(raw_series.len());
    for raw in raw_series {
        validate_id(manifest_path, "scaling series", &raw.id)?;
        if !all_ids.insert(raw.id.clone()) {
            return Err(invalid(
                manifest_path,
                format!("scaling series '{}'", raw.id),
                "duplicate global ID",
            ));
        }

        let subject = format!("scaling series '{}'", raw.id);

        if raw.metric.trim().is_empty() {
            return Err(invalid(manifest_path, subject, "metric must not be empty"));
        }
        if !raw.max_exponent.is_finite() || raw.max_exponent <= 0.0 {
            return Err(invalid(
                manifest_path,
                subject,
                format!(
                    "max_exponent must be a finite positive number, got {}",
                    raw.max_exponent
                ),
            ));
        }
        if raw.points.len() < MINIMUM_SCALING_POINTS {
            return Err(invalid(
                manifest_path,
                subject,
                format!(
                    "must declare at least {MINIMUM_SCALING_POINTS} points, got {}",
                    raw.points.len()
                ),
            ));
        }

        let mut points = Vec::with_capacity(raw.points.len());
        let mut previous_size = 0u32;
        let mut member_runner: Option<&BenchmarkRunner> = None;
        for raw_point in raw.points {
            let Some(&case_index) = case_indexes.get(raw_point.case.as_str()) else {
                return Err(invalid(
                    manifest_path,
                    subject,
                    format!("unknown case '{}'", raw_point.case),
                ));
            };
            if raw_point.size <= previous_size {
                return Err(invalid(
                    manifest_path,
                    subject,
                    format!(
                        "point sizes must strictly increase, got {} after {}",
                        raw_point.size, previous_size
                    ),
                ));
            }
            previous_size = raw_point.size;

            // Every point must exercise the same runner, or the fit compares
            // two different measurements and calls the difference growth.
            let runner = &cases[case_index].runner;
            match member_runner {
                None => member_runner = Some(runner),
                Some(first) if first == runner => {}
                Some(_) => {
                    return Err(invalid(
                        manifest_path,
                        subject,
                        format!(
                            "case '{}' declares a different runner from the first point",
                            raw_point.case
                        ),
                    ));
                }
            }

            points.push(BenchmarkScalingPoint {
                case_id: raw_point.case,
                case_index,
                size: raw_point.size,
            });
        }

        series_list.push(BenchmarkScalingSeries {
            id: raw.id,
            metric: raw.metric,
            max_exponent: raw.max_exponent,
            points,
        });
    }

    Ok(series_list)
}

#[derive(Debug, Hash, PartialEq, Eq)]
enum InvocationKey {
    Cli {
        workload_index: usize,
        command: CliBenchmarkCommand,
        args: Vec<String>,
    },
    Frontend {
        workload_index: usize,
        profile: FrontendBenchmarkProfile,
    },
}

fn validate_id(manifest_path: &Path, kind: &str, id: &str) -> Result<(), BenchmarkManifestError> {
    let bytes = id.as_bytes();
    let valid_characters = bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_');
    if id.is_empty()
        || !valid_characters
        || bytes.first() == Some(&b'_')
        || bytes.last() == Some(&b'_')
    {
        return Err(invalid(
            manifest_path,
            format!("{kind} '{id}'"),
            "ID must contain only lowercase ASCII letters, digits and underscores without leading or trailing underscores",
        ));
    }
    Ok(())
}

fn validate_existing_workload_path(
    manifest_path: &Path,
    repository_root: &Path,
    workload_id: &str,
    field: &'static str,
    authored_path: &str,
) -> Result<ValidatedWorkloadPath, BenchmarkManifestError> {
    let relative_path = validate_relative_path(
        manifest_path,
        format!("workload '{workload_id}'"),
        field,
        authored_path,
    )?;
    let absolute_path = repository_root.join(&relative_path);
    let canonical = canonicalize_workload_path(
        manifest_path,
        workload_id,
        field,
        authored_path,
        &absolute_path,
    )?;

    if !canonical.starts_with(repository_root) {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            format!("{field} '{authored_path}' escapes the repository root after canonicalisation"),
        ));
    }
    let is_directory = fs::metadata(&canonical)
        .map_err(|source| {
            workload_path_error(manifest_path, workload_id, field, authored_path, source)
        })?
        .is_dir();

    Ok(ValidatedWorkloadPath {
        relative: relative_path,
        canonical,
        is_directory,
    })
}

fn validate_relative_path(
    manifest_path: &Path,
    subject: String,
    path_kind: &str,
    raw_path: &str,
) -> Result<PathBuf, BenchmarkManifestError> {
    if raw_path.trim().is_empty() {
        return Err(invalid(
            manifest_path,
            subject,
            format!("{path_kind} must not be empty"),
        ));
    }
    if has_platform_prefix(raw_path) {
        return Err(invalid(
            manifest_path,
            subject,
            format!("{path_kind} '{raw_path}' must be repository-relative"),
        ));
    }
    if raw_path
        .split(['/', '\\'])
        .any(|component| matches!(component, "." | ".."))
    {
        return Err(invalid(
            manifest_path,
            subject,
            format!("{path_kind} '{raw_path}' may not contain '.' or '..' components"),
        ));
    }

    let portable_path = raw_path.replace('\\', "/");
    let path = PathBuf::from(&portable_path);
    if path.is_absolute() {
        return Err(invalid(
            manifest_path,
            subject,
            format!("{path_kind} '{raw_path}' must be repository-relative"),
        ));
    }
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err(invalid(
                manifest_path,
                subject,
                format!("{path_kind} '{raw_path}' may not contain '.' or '..' components"),
            ));
        }
    }
    Ok(path)
}

fn has_platform_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn validate_fingerprint_exclude(
    manifest_path: &Path,
    repository_root: &Path,
    workload_id: &str,
    authored_path: &str,
    fingerprint_roots: &[ValidatedWorkloadPath],
    seen_excludes: &[PathBuf],
) -> Result<PathBuf, BenchmarkManifestError> {
    const FIELD: &str = "fingerprint exclude";

    let relative_path = validate_relative_path(
        manifest_path,
        format!("workload '{workload_id}'"),
        FIELD,
        authored_path,
    )?;

    // Reject duplicate excludes.
    if seen_excludes.iter().any(|seen| seen == &relative_path) {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            format!("duplicate fingerprint exclude '{authored_path}'"),
        ));
    }

    // Reject an exclude equal to a declared root.
    if fingerprint_roots
        .iter()
        .any(|root| root.relative == relative_path)
    {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            format!("fingerprint exclude '{authored_path}' is equal to a declared root"),
        ));
    }

    let containing_root = fingerprint_roots
        .iter()
        .filter(|root| {
            root.is_directory
                && relative_path != root.relative
                && relative_path.starts_with(&root.relative)
        })
        .max_by_key(|root| root.relative.components().count());
    let Some(containing_root) = containing_root else {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            format!(
                "fingerprint exclude '{authored_path}' must be a strict descendant of a declared directory root"
            ),
        ));
    };

    // Reject an exclude that contains another declared root.
    for root in fingerprint_roots {
        if root.relative != containing_root.relative
            && root.relative.starts_with(&relative_path)
            && relative_path != root.relative
        {
            return Err(invalid(
                manifest_path,
                format!("workload '{workload_id}'"),
                format!(
                    "fingerprint exclude '{authored_path}' contains another declared root '{}'",
                    root.relative.display()
                ),
            ));
        }
    }

    let absolute_path = repository_root.join(&relative_path);
    let existing_path = nearest_existing_path(&absolute_path).map_err(|source| {
        workload_path_error(manifest_path, workload_id, FIELD, authored_path, source)
    })?;
    let canonical_existing = canonicalize_workload_path(
        manifest_path,
        workload_id,
        FIELD,
        authored_path,
        &existing_path,
    )?;
    let resolves_to_root =
        existing_path == absolute_path && canonical_existing == containing_root.canonical;
    if resolves_to_root || !canonical_existing.starts_with(&containing_root.canonical) {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            format!("fingerprint exclude '{authored_path}' escapes its declared directory root"),
        ));
    }

    Ok(relative_path)
}

fn canonicalize_workload_path(
    manifest_path: &Path,
    workload_id: &str,
    field: &'static str,
    authored_path: &str,
    path: &Path,
) -> Result<PathBuf, BenchmarkManifestError> {
    fs::canonicalize(path).map_err(|source| {
        workload_path_error(manifest_path, workload_id, field, authored_path, source)
    })
}

fn workload_path_error(
    manifest_path: &Path,
    workload_id: &str,
    field: &'static str,
    authored_path: &str,
    source: io::Error,
) -> BenchmarkManifestError {
    BenchmarkManifestError::WorkloadPath {
        manifest_path: manifest_path.to_owned(),
        workload_id: workload_id.to_owned(),
        field,
        authored_path: authored_path.to_owned(),
        source,
    }
}

/// Validate declared generated output roots for one workload.
///
/// Roots are cleanup authority only. File workloads declare none; directory
/// roots must be strict descendants of the entry, disjoint (including ASCII
/// case collisions), non-symlink when present, and each covered by an exact
/// fingerprint exclude. The roots are expected to be absent at run start, so
/// they are validated logically rather than as existing workload paths.
fn validate_generated_output_roots(
    manifest_path: &Path,
    workload_id: &str,
    entry: &ValidatedWorkloadPath,
    authored_roots: &[String],
    fingerprint_excludes: &[PathBuf],
) -> Result<Vec<PathBuf>, BenchmarkManifestError> {
    const FIELD: &str = "generated output root";

    if !entry.is_directory && !authored_roots.is_empty() {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            "file workloads may not declare generated output roots",
        ));
    }

    let mut roots: Vec<PathBuf> = Vec::with_capacity(authored_roots.len());
    let mut folded_roots: Vec<Vec<String>> = Vec::with_capacity(authored_roots.len());
    for authored_root in authored_roots {
        let relative_path = validate_relative_path(
            manifest_path,
            format!("workload '{workload_id}'"),
            FIELD,
            authored_root,
        )?;

        // Roots are entry-relative, so any validated relative path is inside
        // the entry by construction; the strict-descendant guarantee comes
        // from the shared relative-path rules (no '.', '..', absolute or
        // platform-prefixed components).

        // Reject duplicate, overlapping and ASCII-case-colliding roots using
        // portable folded component vectors so case-insensitive filesystems
        // cannot accept physically overlapping spellings.
        let folded = folded_path_components(&relative_path);
        for (existing, existing_folded) in roots.iter().zip(&folded_roots) {
            if existing == &relative_path {
                return Err(invalid(
                    manifest_path,
                    format!("workload '{workload_id}'"),
                    format!("duplicate generated output root '{authored_root}'"),
                ));
            }
            if existing_folded == &folded {
                return Err(invalid(
                    manifest_path,
                    format!("workload '{workload_id}'"),
                    format!(
                        "generated output root '{authored_root}' differs from '{}' only by ASCII case",
                        existing.display()
                    ),
                ));
            }
            if components_are_prefix(&folded, existing_folded)
                || components_are_prefix(existing_folded, &folded)
            {
                return Err(invalid(
                    manifest_path,
                    format!("workload '{workload_id}'"),
                    format!(
                        "generated output roots '{}' and '{authored_root}' must not overlap",
                        existing.display()
                    ),
                ));
            }
        }

        // A root that already exists must not be a symlink.
        let absolute_path = entry.canonical.join(&relative_path);
        if let Ok(metadata) = std::fs::symlink_metadata(&absolute_path)
            && metadata.file_type().is_symlink()
        {
            return Err(invalid(
                manifest_path,
                format!("workload '{workload_id}'"),
                format!("generated output root '{authored_root}' must not be a symlink"),
            ));
        }

        // Each root must be covered by an exact fingerprint exclude.
        let exclude = entry.relative.join(&relative_path);
        if !fingerprint_excludes.contains(&exclude) {
            return Err(invalid(
                manifest_path,
                format!("workload '{workload_id}'"),
                format!(
                    "generated output root '{authored_root}' must be covered by an explicit fingerprint exclude '{}'",
                    exclude.display()
                ),
            ));
        }

        roots.push(relative_path.clone());
        folded_roots.push(folded);
    }

    // Stable ordering by folded components so deletion and reporting never
    // depend on authored spelling or insertion order.
    let mut indexed: Vec<(Vec<String>, PathBuf)> = folded_roots.into_iter().zip(roots).collect();
    indexed.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(indexed.into_iter().map(|(_, path)| path).collect())
}

/// Fold one validated entry-relative path into portable ASCII-lowercase
/// component strings for case-insensitive comparison and ordering.
fn folded_path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Whether one folded component vector is a strict prefix of another.
fn components_are_prefix(prefix: &[String], full: &[String]) -> bool {
    prefix.len() < full.len() && full.starts_with(prefix)
}

/// Validate fingerprint roots against the declared boundary mode.
///
/// `full_tree` requires exactly one root that resolves to the entry itself.
/// `partitioned` requires every root to be a strict descendant of the entry
/// directory and rejects overlapping or duplicate roots.
fn validate_fingerprint_mode(
    manifest_path: &Path,
    workload_id: &str,
    mode: BenchmarkFingerprintMode,
    entry: &ValidatedWorkloadPath,
    roots: &[ValidatedWorkloadPath],
) -> Result<(), BenchmarkManifestError> {
    match mode {
        BenchmarkFingerprintMode::FullTree => {
            validate_full_tree(manifest_path, workload_id, entry, roots)
        }
        BenchmarkFingerprintMode::Partitioned => {
            validate_partitioned(manifest_path, workload_id, entry, roots)
        }
    }
}

fn validate_full_tree(
    manifest_path: &Path,
    workload_id: &str,
    entry: &ValidatedWorkloadPath,
    roots: &[ValidatedWorkloadPath],
) -> Result<(), BenchmarkManifestError> {
    if roots.len() != 1 {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            "full_tree mode requires exactly one fingerprint root",
        ));
    }

    let root = &roots[0];
    if root.canonical != entry.canonical {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            "full_tree mode requires the fingerprint root to resolve to the entry",
        ));
    }

    Ok(())
}

fn validate_partitioned(
    manifest_path: &Path,
    workload_id: &str,
    entry: &ValidatedWorkloadPath,
    roots: &[ValidatedWorkloadPath],
) -> Result<(), BenchmarkManifestError> {
    if !entry.is_directory {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            "partitioned mode requires a directory entry",
        ));
    }

    if roots.is_empty() {
        return Err(invalid(
            manifest_path,
            format!("workload '{workload_id}'"),
            "partitioned mode requires at least one fingerprint root",
        ));
    }

    // Reject duplicate logical or canonical roots.
    for (i, left) in roots.iter().enumerate() {
        for (_j, right) in roots.iter().enumerate().skip(i + 1) {
            if left.relative == right.relative || left.canonical == right.canonical {
                return Err(invalid(
                    manifest_path,
                    format!("workload '{workload_id}'"),
                    "partitioned mode rejects duplicate fingerprint roots",
                ));
            }
        }
    }

    for root in roots {
        // Every root must be a strict descendant of the entry directory.
        if root.canonical == entry.canonical || !root.canonical.starts_with(&entry.canonical) {
            return Err(invalid(
                manifest_path,
                format!("workload '{workload_id}'"),
                "partitioned mode requires every root to be a strict descendant of the entry directory",
            ));
        }

        // Reject root pairs where either root contains the other.
        for other in roots {
            if other.relative == root.relative {
                continue;
            }
            if root.canonical.starts_with(&other.canonical)
                || other.canonical.starts_with(&root.canonical)
            {
                return Err(invalid(
                    manifest_path,
                    format!("workload '{workload_id}'"),
                    "partitioned mode rejects ancestor or descendant root pairs",
                ));
            }
        }
    }

    Ok(())
}

fn nearest_existing_path(path: &Path) -> io::Result<PathBuf> {
    let mut candidate = path.to_owned();

    loop {
        match fs::symlink_metadata(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                if !candidate.pop() {
                    return Err(source);
                }
            }
            Err(source) => return Err(source),
        }
    }
}

fn invalid(
    manifest_path: &Path,
    subject: impl Into<String>,
    message: impl Into<String>,
) -> BenchmarkManifestError {
    BenchmarkManifestError::Invalid {
        path: manifest_path.to_owned(),
        subject: subject.into(),
        message: message.into(),
    }
}

fn find_repository_root(start: &Path) -> Option<PathBuf> {
    let mut candidate = start;
    loop {
        if candidate.join(BENCHMARK_MANIFEST_PATH).is_file() {
            return Some(candidate.to_owned());
        }
        candidate = candidate.parent()?;
    }
}

#[cfg(test)]
#[path = "benchmark_manifest/tests.rs"]
mod tests;
