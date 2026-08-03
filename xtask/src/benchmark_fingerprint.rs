//! Deterministic identity for benchmark source workloads and case measurements.
//!
//! WHAT: Computes one source workload fingerprint per workload covering only
//! authored source inputs, and one case measurement fingerprint per case
//! covering the source fingerprint plus benchmark protocol, runner kind,
//! command, arguments and expectation.
//! WHY: Changing one case's runner must not invalidate another case attached
//! to the same workload. Changing source bytes must invalidate every case
//! attached to that workload. This hash detects change deterministically; it
//! does not provide cryptographic security.

use crate::bench_types::{BENCHMARK_PROTOCOL_VERSION, BenchmarkMeasurementIdentity};
use crate::benchmark_manifest::{
    BENCHMARK_MANIFEST_SCHEMA_VERSION, BenchmarkCase, BenchmarkExpectation,
    BenchmarkFingerprintMode, BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload,
};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

const SOURCE_FINGERPRINT_VERSION: u32 = 2;
const MEASUREMENT_FINGERPRINT_VERSION: u32 = 1;
const FNV_1A_OFFSET_BASIS_64: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_1A_PRIME_64: u64 = 0x0000_0100_0000_01b3;

/// Stable two-lane FNV-1a fingerprint for one workload's source inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SourceWorkloadFingerprint {
    first_lane: u64,
    second_lane: u64,
}

/// Stable two-lane FNV-1a fingerprint for one case's measurement identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CaseMeasurementFingerprint {
    first_lane: u64,
    second_lane: u64,
}

/// Combined fingerprints for one manifest, computed once per command.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkFingerprints {
    pub(crate) workloads: Vec<SourceWorkloadFingerprint>,
    pub(crate) cases: Vec<CaseMeasurementFingerprint>,
}

impl BenchmarkFingerprints {
    /// Construct the checked measurement identity for one case.
    ///
    /// The single identity helper shared by CLI, frontend and profile paths.
    /// It fails when the workload relationship or either fingerprint lane is
    /// missing instead of silently degrading to an optional identity.
    pub(crate) fn identity_for(
        &self,
        manifest: &BenchmarkManifest,
        case: &BenchmarkCase,
    ) -> Result<BenchmarkMeasurementIdentity, BenchmarkIdentityError> {
        let workload = manifest.workload_for(case).ok_or_else(|| {
            BenchmarkIdentityError::InvalidWorkloadRelationship {
                case_id: case.id.clone(),
            }
        })?;
        let source_fingerprint = self.workloads.get(case.workload_index).ok_or_else(|| {
            BenchmarkIdentityError::MissingWorkloadFingerprint {
                case_id: case.id.clone(),
                workload_index: case.workload_index,
            }
        })?;
        let measurement_fingerprint = self.cases.get(case.case_index).ok_or_else(|| {
            BenchmarkIdentityError::MissingMeasurementFingerprint {
                case_id: case.id.clone(),
                case_index: case.case_index,
            }
        })?;

        Ok(BenchmarkMeasurementIdentity {
            workload_id: workload.id.clone(),
            source_fingerprint: source_fingerprint.to_string(),
            measurement_fingerprint: measurement_fingerprint.to_string(),
        })
    }
}

impl Display for SourceWorkloadFingerprint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:016x}{:016x}",
            self.first_lane, self.second_lane
        )
    }
}

impl Display for CaseMeasurementFingerprint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:016x}{:016x}",
            self.first_lane, self.second_lane
        )
    }
}

/// Failures while constructing one checked measurement identity.
#[derive(Debug)]
pub(crate) enum BenchmarkIdentityError {
    /// The case does not resolve to a declared workload in the manifest.
    InvalidWorkloadRelationship { case_id: String },
    /// The workload fingerprint lane is missing for the case's workload index.
    MissingWorkloadFingerprint {
        case_id: String,
        workload_index: usize,
    },
    /// The measurement fingerprint lane is missing for the case's case index.
    MissingMeasurementFingerprint { case_id: String, case_index: usize },
}

impl Display for BenchmarkIdentityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWorkloadRelationship { case_id } => write!(
                formatter,
                "benchmark case '{case_id}' has no valid workload relationship"
            ),
            Self::MissingWorkloadFingerprint {
                case_id,
                workload_index,
            } => write!(
                formatter,
                "benchmark case '{case_id}' has no source fingerprint for workload index {workload_index}"
            ),
            Self::MissingMeasurementFingerprint {
                case_id,
                case_index,
            } => write!(
                formatter,
                "benchmark case '{case_id}' has no measurement fingerprint for case index {case_index}"
            ),
        }
    }
}

impl std::error::Error for BenchmarkIdentityError {}

/// Contextual failures while resolving and reading workload inputs.
#[derive(Debug)]
pub(crate) enum BenchmarkFingerprintError {
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

impl Display for BenchmarkFingerprintError {
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

impl std::error::Error for BenchmarkFingerprintError {
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

/// Compute all source and measurement fingerprints exactly once in manifest order.
pub(crate) fn compute_benchmark_fingerprints(
    manifest: &BenchmarkManifest,
) -> Result<BenchmarkFingerprints, BenchmarkFingerprintError> {
    let canonical_repository_root =
        fs::canonicalize(&manifest.repository_root).map_err(|source| {
            BenchmarkFingerprintError::RepositoryRoot {
                path: manifest.repository_root.clone(),
                source,
            }
        })?;

    let mut workload_fingerprints = Vec::with_capacity(manifest.workloads.len());
    for workload in &manifest.workloads {
        let fingerprint = compute_source_fingerprint(workload, &canonical_repository_root)?;
        workload_fingerprints.push(fingerprint);
    }

    let mut case_fingerprints = Vec::with_capacity(manifest.cases.len());
    for case in &manifest.cases {
        let Some(workload) = manifest.workloads.get(case.workload_index) else {
            return Err(BenchmarkFingerprintError::InvalidWorkloadReference {
                case_id: case.id.clone(),
                workload_index: case.workload_index,
            });
        };
        let source_fingerprint = workload_fingerprints[case.workload_index];
        let measurement = compute_measurement_fingerprint(source_fingerprint, workload, case);
        case_fingerprints.push(measurement);
    }

    Ok(BenchmarkFingerprints {
        workloads: workload_fingerprints,
        cases: case_fingerprints,
    })
}

fn compute_source_fingerprint(
    workload: &BenchmarkWorkload,
    repository_root: &Path,
) -> Result<SourceWorkloadFingerprint, BenchmarkFingerprintError> {
    let included_files = collect_included_files(workload, repository_root)?;
    let mut fingerprint = FingerprintBuilder::new();

    fingerprint.write_field(b"moth.source-workload-fingerprint");
    fingerprint.write_u32(SOURCE_FINGERPRINT_VERSION);
    fingerprint.write_u32(BENCHMARK_MANIFEST_SCHEMA_VERSION);

    hash_workload_declaration(&mut fingerprint, workload)?;

    fingerprint.write_usize(included_files.len());
    for (logical_path, absolute_path) in included_files {
        let bytes = read_included_file(workload, repository_root, &logical_path, &absolute_path)?;
        fingerprint.write_field(logical_path.as_bytes());
        fingerprint.write_field(&bytes);
    }

    Ok(fingerprint.finish_source())
}

fn compute_measurement_fingerprint(
    source_fingerprint: SourceWorkloadFingerprint,
    workload: &BenchmarkWorkload,
    case: &BenchmarkCase,
) -> CaseMeasurementFingerprint {
    let mut fingerprint = FingerprintBuilder::new();

    fingerprint.write_field(b"moth.case-measurement-fingerprint");
    fingerprint.write_u32(MEASUREMENT_FINGERPRINT_VERSION);
    fingerprint.write_u32(BENCHMARK_PROTOCOL_VERSION);

    fingerprint.write_field(&source_fingerprint.first_lane.to_le_bytes());
    fingerprint.write_field(&source_fingerprint.second_lane.to_le_bytes());

    fingerprint.write_field(workload.id.as_bytes());

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

    fingerprint.write_field(match case.expectation {
        BenchmarkExpectation::Clean => b"clean",
    });

    fingerprint.finish_measurement()
}

fn hash_workload_declaration(
    fingerprint: &mut FingerprintBuilder,
    workload: &BenchmarkWorkload,
) -> Result<(), BenchmarkFingerprintError> {
    fingerprint.write_field(normalized_path(workload, &workload.entry)?.as_bytes());

    fingerprint.write_field(match workload.entry_kind {
        crate::benchmark_manifest::BenchmarkEntryKind::File => b"file",
        crate::benchmark_manifest::BenchmarkEntryKind::Directory => b"directory",
    });

    fingerprint.write_field(match workload.fingerprint_mode {
        BenchmarkFingerprintMode::FullTree => b"full_tree",
        BenchmarkFingerprintMode::Partitioned => b"partitioned",
    });

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
) -> Result<BTreeMap<String, PathBuf>, BenchmarkFingerprintError> {
    let mut included_files = BTreeMap::new();

    for root in &workload.fingerprint_roots {
        let root_metadata = inspect_root_path(workload, repository_root, root)?;
        let absolute_root = repository_root.join(root);
        let canonical_root = fs::canonicalize(&absolute_root).map_err(|source| {
            BenchmarkFingerprintError::RootAccess {
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
            return Err(BenchmarkFingerprintError::UnsupportedFileType {
                workload_id: workload.id.clone(),
                path: root.clone(),
            });
        }
    }

    if included_files.is_empty() {
        return Err(BenchmarkFingerprintError::EmptyFileSet {
            workload_id: workload.id.clone(),
        });
    }

    Ok(included_files)
}

fn inspect_root_path(
    workload: &BenchmarkWorkload,
    repository_root: &Path,
    root: &Path,
) -> Result<fs::Metadata, BenchmarkFingerprintError> {
    let mut current_absolute = repository_root.to_owned();
    let mut current_logical = PathBuf::new();
    let mut root_metadata = None;

    for component in root.components() {
        let Component::Normal(name) = component else {
            return Err(BenchmarkFingerprintError::InvalidLogicalPath {
                workload_id: workload.id.clone(),
                path: root.to_owned(),
                reason: "path must contain repository-relative normal components only",
            });
        };

        current_absolute.push(name);
        current_logical.push(name);
        let metadata = fs::symlink_metadata(&current_absolute).map_err(|source| {
            BenchmarkFingerprintError::RootAccess {
                workload_id: workload.id.clone(),
                path: current_logical.clone(),
                source,
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(BenchmarkFingerprintError::Symlink {
                workload_id: workload.id.clone(),
                path: current_logical,
            });
        }
        root_metadata = Some(metadata);
    }

    root_metadata.ok_or_else(|| BenchmarkFingerprintError::InvalidLogicalPath {
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
) -> Result<(), BenchmarkFingerprintError> {
    let entries = fs::read_dir(absolute_directory).map_err(|source| {
        BenchmarkFingerprintError::DirectoryRead {
            workload_id: workload.id.clone(),
            path: logical_directory.to_owned(),
            source,
        }
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| BenchmarkFingerprintError::DirectoryRead {
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
                .map_err(|source| BenchmarkFingerprintError::FileAccess {
                    workload_id: workload.id.clone(),
                    path: logical_path.clone(),
                    source,
                })?;

        if file_type.is_symlink() {
            return Err(BenchmarkFingerprintError::Symlink {
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
                BenchmarkFingerprintError::FileAccess {
                    workload_id: workload.id.clone(),
                    path: logical_path.clone(),
                    source,
                }
            })?;
            ensure_inside_repository(workload, &logical_path, &canonical_file, repository_root)?;
            let normalized_path = normalized_path(workload, &logical_path)?;
            included_files.insert(normalized_path, canonical_file);
        } else if !file_type.is_file() {
            return Err(BenchmarkFingerprintError::UnsupportedFileType {
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
) -> Result<Vec<u8>, BenchmarkFingerprintError> {
    let logical_path = PathBuf::from(logical_path);
    let metadata = fs::symlink_metadata(absolute_path).map_err(|source| {
        BenchmarkFingerprintError::FileAccess {
            workload_id: workload.id.clone(),
            path: logical_path.clone(),
            source,
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(BenchmarkFingerprintError::Symlink {
            workload_id: workload.id.clone(),
            path: logical_path,
        });
    }
    if !metadata.is_file() {
        return Err(BenchmarkFingerprintError::UnsupportedFileType {
            workload_id: workload.id.clone(),
            path: logical_path,
        });
    }

    let canonical_file = fs::canonicalize(absolute_path).map_err(|source| {
        BenchmarkFingerprintError::FileAccess {
            workload_id: workload.id.clone(),
            path: logical_path.clone(),
            source,
        }
    })?;
    ensure_inside_repository(workload, &logical_path, &canonical_file, repository_root)?;

    fs::read(&canonical_file).map_err(|source| BenchmarkFingerprintError::FileRead {
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
) -> Result<(), BenchmarkFingerprintError> {
    if canonical_path.starts_with(repository_root) {
        return Ok(());
    }

    Err(BenchmarkFingerprintError::RepositoryEscape {
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
) -> Result<String, BenchmarkFingerprintError> {
    let mut components = Vec::new();

    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(BenchmarkFingerprintError::InvalidLogicalPath {
                workload_id: workload.id.clone(),
                path: path.to_owned(),
                reason: "path must contain repository-relative normal components only",
            });
        };
        let Some(name) = name.to_str() else {
            return Err(BenchmarkFingerprintError::NonUnicodePath {
                workload_id: workload.id.clone(),
                path: path.to_owned(),
            });
        };
        components.push(name);
    }

    if components.is_empty() {
        return Err(BenchmarkFingerprintError::InvalidLogicalPath {
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

    fn finish_source(self) -> SourceWorkloadFingerprint {
        SourceWorkloadFingerprint {
            first_lane: self.lanes[0],
            second_lane: self.lanes[1],
        }
    }

    fn finish_measurement(self) -> CaseMeasurementFingerprint {
        CaseMeasurementFingerprint {
            first_lane: self.lanes[0],
            second_lane: self.lanes[1],
        }
    }
}

#[cfg(test)]
#[path = "benchmark_fingerprint/tests.rs"]
mod tests;
