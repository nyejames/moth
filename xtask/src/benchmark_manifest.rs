//! Typed authority for the repository benchmark inventory.
//!
//! WHAT: Loads the schema-1 TOML manifest, validates authored identities and
//! resolves each case to one immutable workload relationship.
//! WHY: Benchmark commands need one strict source of case order, runner
//! semantics and filesystem ownership instead of path-derived text lists.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub(crate) const BENCHMARK_MANIFEST_PATH: &str = "benchmarks/manifest.toml";
pub(crate) const BENCHMARK_MANIFEST_SCHEMA_VERSION: u32 = 2;

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
    pub(crate) manifest_path: PathBuf,
    pub(crate) repository_root: PathBuf,
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
}

/// One authored case with a compact index into the manifest workloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BenchmarkCase {
    pub(crate) id: String,
    pub(crate) workload_index: usize,
    pub(crate) group_name: String,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBenchmarkWorkload {
    id: String,
    entry: String,
    fingerprint_mode: BenchmarkFingerprintMode,
    fingerprint_roots: Vec<String>,
    fingerprint_excludes: Vec<String>,
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

fn load_benchmark_manifest_from(
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
        if raw_case.group.trim().is_empty() {
            return Err(invalid(
                manifest_path,
                format!("case '{}'", raw_case.id),
                "group must not be empty",
            ));
        }
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

        cases.push(BenchmarkCase {
            id: raw_case.id,
            workload_index,
            group_name: raw_case.group,
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

    Ok(BenchmarkManifest {
        workloads,
        cases,
        manifest_path: manifest_path.to_owned(),
        repository_root: repository_root.to_owned(),
    })
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
        for (j, right) in roots.iter().enumerate().skip(i + 1) {
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
