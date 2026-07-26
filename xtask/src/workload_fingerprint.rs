//! Deterministic identity for benchmark workloads and their runner protocols.
//!
//! WHAT: Hashes one workload's authored inputs, complete runner declarations
//! and included repository files into a stable 128-bit fingerprint.
//! WHY: Later benchmark preflight and history phases must distinguish source or
//! methodology changes from compiler performance changes. This hash detects
//! change deterministically; it does not provide cryptographic security.

use crate::bench_types::BENCHMARK_PROTOCOL_VERSION;
use crate::benchmark_manifest::{
    BENCHMARK_MANIFEST_SCHEMA_VERSION, BenchmarkCase, BenchmarkManifest, BenchmarkRunner,
    BenchmarkWorkload,
};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

const WORKLOAD_FINGERPRINT_VERSION: u32 = 1;
const FNV_1A_OFFSET_BASIS_64: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_1A_PRIME_64: u64 = 0x0000_0100_0000_01b3;

/// Stable two-lane FNV-1a fingerprint for one manifest workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorkloadFingerprint {
    first_lane: u64,
    second_lane: u64,
}

impl Display for WorkloadFingerprint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:016x}{:016x}",
            self.first_lane, self.second_lane
        )
    }
}

/// Contextual failures while resolving and reading workload inputs.
#[derive(Debug)]
pub(crate) enum WorkloadFingerprintError {
    RepositoryRoot {
        path: PathBuf,
        source: io::Error,
    },
    InvalidWorkloadReference {
        case_id: String,
        workload_index: usize,
    },
    InvalidLogicalPath {
        workload_id: String,
        path: PathBuf,
        reason: &'static str,
    },
    RootAccess {
        workload_id: String,
        path: PathBuf,
        source: io::Error,
    },
    RepositoryEscape {
        workload_id: String,
        path: PathBuf,
        repository_root: PathBuf,
    },
    Symlink {
        workload_id: String,
        path: PathBuf,
    },
    DirectoryRead {
        workload_id: String,
        path: PathBuf,
        source: io::Error,
    },
    FileAccess {
        workload_id: String,
        path: PathBuf,
        source: io::Error,
    },
    FileRead {
        workload_id: String,
        path: PathBuf,
        source: io::Error,
    },
    UnsupportedFileType {
        workload_id: String,
        path: PathBuf,
    },
    NonUnicodePath {
        workload_id: String,
        path: PathBuf,
    },
    EmptyFileSet {
        workload_id: String,
    },
}

impl Display for WorkloadFingerprintError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RepositoryRoot { path, source } => write!(
                formatter,
                "failed to canonicalise benchmark repository root '{}': {source}",
                path.display()
            ),
            Self::InvalidWorkloadReference {
                case_id,
                workload_index,
            } => write!(
                formatter,
                "benchmark case '{case_id}' references missing workload index {workload_index}"
            ),
            Self::InvalidLogicalPath {
                workload_id,
                path,
                reason,
            } => write!(
                formatter,
                "invalid fingerprint path '{}' for workload '{workload_id}': {reason}",
                path.display()
            ),
            Self::RootAccess {
                workload_id,
                path,
                source,
            } => write!(
                formatter,
                "failed to inspect fingerprint root '{}' for workload '{workload_id}': {source}",
                path.display()
            ),
            Self::RepositoryEscape {
                workload_id,
                path,
                repository_root,
            } => write!(
                formatter,
                "fingerprint path '{}' for workload '{workload_id}' escapes repository root '{}'",
                path.display(),
                repository_root.display()
            ),
            Self::Symlink { workload_id, path } => write!(
                formatter,
                "fingerprint path '{}' for workload '{workload_id}' is a symlink; workload fingerprints reject symlinks",
                path.display()
            ),
            Self::DirectoryRead {
                workload_id,
                path,
                source,
            } => write!(
                formatter,
                "failed to read fingerprint directory '{}' for workload '{workload_id}': {source}",
                path.display()
            ),
            Self::FileAccess {
                workload_id,
                path,
                source,
            } => write!(
                formatter,
                "failed to inspect fingerprint file '{}' for workload '{workload_id}': {source}",
                path.display()
            ),
            Self::FileRead {
                workload_id,
                path,
                source,
            } => write!(
                formatter,
                "failed to read fingerprint file '{}' for workload '{workload_id}': {source}",
                path.display()
            ),
            Self::UnsupportedFileType { workload_id, path } => write!(
                formatter,
                "fingerprint path '{}' for workload '{workload_id}' is not a regular file or directory",
                path.display()
            ),
            Self::NonUnicodePath { workload_id, path } => write!(
                formatter,
                "fingerprint path '{}' for workload '{workload_id}' is not valid Unicode",
                path.display()
            ),
            Self::EmptyFileSet { workload_id } => write!(
                formatter,
                "fingerprint roots for workload '{workload_id}' contain no included files"
            ),
        }
    }
}

impl std::error::Error for WorkloadFingerprintError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RepositoryRoot { source, .. }
            | Self::RootAccess { source, .. }
            | Self::DirectoryRead { source, .. }
            | Self::FileAccess { source, .. }
            | Self::FileRead { source, .. } => Some(source),
            Self::InvalidWorkloadReference { .. }
            | Self::InvalidLogicalPath { .. }
            | Self::RepositoryEscape { .. }
            | Self::Symlink { .. }
            | Self::UnsupportedFileType { .. }
            | Self::NonUnicodePath { .. }
            | Self::EmptyFileSet { .. } => None,
        }
    }
}

/// Compute every workload fingerprint exactly once in manifest workload order.
pub(crate) fn compute_workload_fingerprints(
    manifest: &BenchmarkManifest,
) -> Result<Vec<WorkloadFingerprint>, WorkloadFingerprintError> {
    let canonical_repository_root =
        fs::canonicalize(&manifest.repository_root).map_err(|source| {
            WorkloadFingerprintError::RepositoryRoot {
                path: manifest.repository_root.clone(),
                source,
            }
        })?;
    let cases_by_workload = cases_by_workload(manifest)?;
    let mut fingerprints = Vec::with_capacity(manifest.workloads.len());

    for (workload_index, workload) in manifest.workloads.iter().enumerate() {
        let fingerprint = compute_workload_fingerprint(
            workload,
            &cases_by_workload[workload_index],
            &canonical_repository_root,
        )?;
        fingerprints.push(fingerprint);
    }

    Ok(fingerprints)
}

fn cases_by_workload(
    manifest: &BenchmarkManifest,
) -> Result<Vec<Vec<&BenchmarkCase>>, WorkloadFingerprintError> {
    let mut cases_by_workload = vec![Vec::new(); manifest.workloads.len()];

    for case in &manifest.cases {
        let Some(workload_cases) = cases_by_workload.get_mut(case.workload_index) else {
            return Err(WorkloadFingerprintError::InvalidWorkloadReference {
                case_id: case.id.clone(),
                workload_index: case.workload_index,
            });
        };
        workload_cases.push(case);
    }

    Ok(cases_by_workload)
}

fn compute_workload_fingerprint(
    workload: &BenchmarkWorkload,
    cases: &[&BenchmarkCase],
    repository_root: &Path,
) -> Result<WorkloadFingerprint, WorkloadFingerprintError> {
    let included_files = collect_included_files(workload, repository_root)?;
    let mut fingerprint = FingerprintBuilder::new();

    fingerprint.write_field(b"moth.workload-fingerprint");
    fingerprint.write_u32(WORKLOAD_FINGERPRINT_VERSION);
    fingerprint.write_u32(BENCHMARK_MANIFEST_SCHEMA_VERSION);
    fingerprint.write_u32(BENCHMARK_PROTOCOL_VERSION);

    hash_runner_declarations(&mut fingerprint, cases);
    hash_workload_declaration(&mut fingerprint, workload)?;

    fingerprint.write_usize(included_files.len());
    for (logical_path, absolute_path) in included_files {
        let bytes = read_included_file(workload, repository_root, &logical_path, &absolute_path)?;
        fingerprint.write_field(logical_path.as_bytes());
        fingerprint.write_field(&bytes);
    }

    Ok(fingerprint.finish())
}

fn hash_runner_declarations(fingerprint: &mut FingerprintBuilder, cases: &[&BenchmarkCase]) {
    fingerprint.write_usize(cases.len());

    for case in cases {
        match &case.runner {
            BenchmarkRunner::Cli { command, args } => {
                fingerprint.write_field(b"cli");
                fingerprint.write_field(command.as_str().as_bytes());
                fingerprint.write_usize(args.len());
                for argument in args {
                    fingerprint.write_field(argument.as_bytes());
                }
            }
            BenchmarkRunner::Frontend { profile } => {
                fingerprint.write_field(b"frontend");
                fingerprint.write_field(profile.as_str().as_bytes());
                fingerprint.write_usize(0);
            }
        }
    }
}

fn hash_workload_declaration(
    fingerprint: &mut FingerprintBuilder,
    workload: &BenchmarkWorkload,
) -> Result<(), WorkloadFingerprintError> {
    fingerprint.write_field(normalized_path(workload, &workload.entry)?.as_bytes());

    fingerprint.write_usize(workload.fingerprint_roots.len());
    for root in &workload.fingerprint_roots {
        fingerprint.write_field(normalized_path(workload, root)?.as_bytes());
    }

    fingerprint.write_usize(workload.fingerprint_excludes.len());
    for exclude in &workload.fingerprint_excludes {
        fingerprint.write_field(normalized_path(workload, exclude)?.as_bytes());
    }

    Ok(())
}

fn collect_included_files(
    workload: &BenchmarkWorkload,
    repository_root: &Path,
) -> Result<BTreeMap<String, PathBuf>, WorkloadFingerprintError> {
    let mut included_files = BTreeMap::new();

    for root in &workload.fingerprint_roots {
        let root_metadata = inspect_root_path(workload, repository_root, root)?;
        let absolute_root = repository_root.join(root);
        let canonical_root = fs::canonicalize(&absolute_root).map_err(|source| {
            WorkloadFingerprintError::RootAccess {
                workload_id: workload.id.clone(),
                path: root.clone(),
                source,
            }
        })?;
        ensure_inside_repository(workload, root, &canonical_root, repository_root)?;

        if root_metadata.is_file() {
            if !is_excluded(root, &workload.fingerprint_excludes) {
                let logical_path = normalized_path(workload, root)?;
                included_files.insert(logical_path, canonical_root);
            }
        } else if root_metadata.is_dir() {
            collect_directory_files(
                workload,
                repository_root,
                root,
                &canonical_root,
                &mut included_files,
            )?;
        } else {
            return Err(WorkloadFingerprintError::UnsupportedFileType {
                workload_id: workload.id.clone(),
                path: root.clone(),
            });
        }
    }

    if included_files.is_empty() {
        return Err(WorkloadFingerprintError::EmptyFileSet {
            workload_id: workload.id.clone(),
        });
    }

    Ok(included_files)
}

fn inspect_root_path(
    workload: &BenchmarkWorkload,
    repository_root: &Path,
    root: &Path,
) -> Result<fs::Metadata, WorkloadFingerprintError> {
    let mut current_absolute = repository_root.to_owned();
    let mut current_logical = PathBuf::new();
    let mut root_metadata = None;

    for component in root.components() {
        let Component::Normal(name) = component else {
            return Err(WorkloadFingerprintError::InvalidLogicalPath {
                workload_id: workload.id.clone(),
                path: root.to_owned(),
                reason: "path must contain repository-relative normal components only",
            });
        };

        current_absolute.push(name);
        current_logical.push(name);
        let metadata = fs::symlink_metadata(&current_absolute).map_err(|source| {
            WorkloadFingerprintError::RootAccess {
                workload_id: workload.id.clone(),
                path: current_logical.clone(),
                source,
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(WorkloadFingerprintError::Symlink {
                workload_id: workload.id.clone(),
                path: current_logical,
            });
        }
        root_metadata = Some(metadata);
    }

    root_metadata.ok_or_else(|| WorkloadFingerprintError::InvalidLogicalPath {
        workload_id: workload.id.clone(),
        path: root.to_owned(),
        reason: "path must not be empty",
    })
}

fn collect_directory_files(
    workload: &BenchmarkWorkload,
    repository_root: &Path,
    logical_directory: &Path,
    absolute_directory: &Path,
    included_files: &mut BTreeMap<String, PathBuf>,
) -> Result<(), WorkloadFingerprintError> {
    let entries = fs::read_dir(absolute_directory).map_err(|source| {
        WorkloadFingerprintError::DirectoryRead {
            workload_id: workload.id.clone(),
            path: logical_directory.to_owned(),
            source,
        }
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| WorkloadFingerprintError::DirectoryRead {
            workload_id: workload.id.clone(),
            path: logical_directory.to_owned(),
            source,
        })?;
        let logical_path = logical_directory.join(entry.file_name());

        // Excluded paths are outside the workload input boundary. Prune them
        // before inspecting their contents so generated outputs cannot affect
        // fingerprint success or identity.
        if is_excluded(&logical_path, &workload.fingerprint_excludes) {
            continue;
        }

        let file_type =
            entry
                .file_type()
                .map_err(|source| WorkloadFingerprintError::FileAccess {
                    workload_id: workload.id.clone(),
                    path: logical_path.clone(),
                    source,
                })?;

        if file_type.is_symlink() {
            return Err(WorkloadFingerprintError::Symlink {
                workload_id: workload.id.clone(),
                path: logical_path,
            });
        }

        if file_type.is_dir() {
            collect_directory_files(
                workload,
                repository_root,
                &logical_path,
                &entry.path(),
                included_files,
            )?;
        } else if file_type.is_file() {
            let canonical_file = fs::canonicalize(entry.path()).map_err(|source| {
                WorkloadFingerprintError::FileAccess {
                    workload_id: workload.id.clone(),
                    path: logical_path.clone(),
                    source,
                }
            })?;
            ensure_inside_repository(workload, &logical_path, &canonical_file, repository_root)?;
            let normalized_path = normalized_path(workload, &logical_path)?;
            included_files.insert(normalized_path, canonical_file);
        } else if !file_type.is_file() {
            return Err(WorkloadFingerprintError::UnsupportedFileType {
                workload_id: workload.id.clone(),
                path: logical_path,
            });
        }
    }

    Ok(())
}

fn read_included_file(
    workload: &BenchmarkWorkload,
    repository_root: &Path,
    logical_path: &str,
    absolute_path: &Path,
) -> Result<Vec<u8>, WorkloadFingerprintError> {
    let logical_path = PathBuf::from(logical_path);
    let metadata = fs::symlink_metadata(absolute_path).map_err(|source| {
        WorkloadFingerprintError::FileAccess {
            workload_id: workload.id.clone(),
            path: logical_path.clone(),
            source,
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(WorkloadFingerprintError::Symlink {
            workload_id: workload.id.clone(),
            path: logical_path,
        });
    }
    if !metadata.is_file() {
        return Err(WorkloadFingerprintError::UnsupportedFileType {
            workload_id: workload.id.clone(),
            path: logical_path,
        });
    }

    let canonical_file =
        fs::canonicalize(absolute_path).map_err(|source| WorkloadFingerprintError::FileAccess {
            workload_id: workload.id.clone(),
            path: logical_path.clone(),
            source,
        })?;
    ensure_inside_repository(workload, &logical_path, &canonical_file, repository_root)?;

    fs::read(&canonical_file).map_err(|source| WorkloadFingerprintError::FileRead {
        workload_id: workload.id.clone(),
        path: logical_path,
        source,
    })
}

fn ensure_inside_repository(
    workload: &BenchmarkWorkload,
    logical_path: &Path,
    canonical_path: &Path,
    repository_root: &Path,
) -> Result<(), WorkloadFingerprintError> {
    if canonical_path.starts_with(repository_root) {
        return Ok(());
    }

    Err(WorkloadFingerprintError::RepositoryEscape {
        workload_id: workload.id.clone(),
        path: logical_path.to_owned(),
        repository_root: repository_root.to_owned(),
    })
}

fn is_excluded(path: &Path, excludes: &[PathBuf]) -> bool {
    excludes.iter().any(|exclude| path.starts_with(exclude))
}

fn normalized_path(
    workload: &BenchmarkWorkload,
    path: &Path,
) -> Result<String, WorkloadFingerprintError> {
    let mut components = Vec::new();

    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(WorkloadFingerprintError::InvalidLogicalPath {
                workload_id: workload.id.clone(),
                path: path.to_owned(),
                reason: "path must contain repository-relative normal components only",
            });
        };
        let Some(name) = name.to_str() else {
            return Err(WorkloadFingerprintError::NonUnicodePath {
                workload_id: workload.id.clone(),
                path: path.to_owned(),
            });
        };
        components.push(name);
    }

    if components.is_empty() {
        return Err(WorkloadFingerprintError::InvalidLogicalPath {
            workload_id: workload.id.clone(),
            path: path.to_owned(),
            reason: "path must not be empty",
        });
    }

    Ok(components.join("/"))
}

struct FingerprintBuilder {
    lanes: [u64; 2],
}

impl FingerprintBuilder {
    fn new() -> Self {
        let mut builder = Self {
            lanes: [FNV_1A_OFFSET_BASIS_64; 2],
        };
        builder.write_lane_domain(0, 0);
        builder.write_lane_domain(1, 1);
        builder
    }

    fn write_field(&mut self, bytes: &[u8]) {
        let length = u64::try_from(bytes.len()).expect("field length should fit into u64");
        self.write_bytes(&length.to_le_bytes());
        self.write_bytes(bytes);
    }

    fn write_u32(&mut self, value: u32) {
        self.write_field(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        let value = u64::try_from(value).expect("field count should fit into u64");
        self.write_field(&value.to_le_bytes());
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            for lane in &mut self.lanes {
                *lane ^= u64::from(*byte);
                *lane = lane.wrapping_mul(FNV_1A_PRIME_64);
            }
        }
    }

    fn write_lane_domain(&mut self, lane_index: usize, byte: u8) {
        self.lanes[lane_index] ^= u64::from(byte);
        self.lanes[lane_index] = self.lanes[lane_index].wrapping_mul(FNV_1A_PRIME_64);
    }

    fn finish(self) -> WorkloadFingerprint {
        WorkloadFingerprint {
            first_lane: self.lanes[0],
            second_lane: self.lanes[1],
        }
    }
}

#[cfg(test)]
#[path = "workload_fingerprint/tests.rs"]
mod tests;
