//! One prepared benchmark run: manifest, repository snapshot, fingerprints and paths.
//!
//! WHAT: Loads the typed manifest, captures the repository snapshot, applies
//! the recording eligibility gate, computes workload and case fingerprints,
//! and anchors every local-data path to the repository root.
//! WHY: Every benchmark, validation and profile mode must observe one source
//! state before fingerprint traversal, compiler construction, system identity
//! creation or persistence, so recording eligibility and identity facts come
//! from one preparation boundary instead of being reconstructed per command.

use crate::bench_types::BenchmarkRecording;
use crate::benchmark_fingerprint::{BenchmarkFingerprints, compute_benchmark_fingerprints};
use crate::benchmark_manifest::{BenchmarkManifest, load_benchmark_manifest_from};
use crate::benchmark_repository::{BenchmarkRepositorySnapshot, require_clean_for_recording};
use std::path::{Path, PathBuf};

/// Repository-anchored local-data and summary paths for one benchmark run.
///
/// Every persistence and reporting owner consumes these paths instead of
/// reconstructing process-relative constants, so direct xtask invocation from
/// a descendant directory cannot read or write a different local-data tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BenchmarkPaths {
    pub(crate) runs_jsonl: PathBuf,
    pub(crate) system_toml: PathBuf,
    pub(crate) summaries: PathBuf,
    pub(crate) profile_history: PathBuf,
    pub(crate) profiles: PathBuf,
}

impl BenchmarkPaths {
    /// Anchor every local benchmark path to the canonical repository root.
    pub(crate) fn for_repository(repository_root: &Path) -> Self {
        let local_data = repository_root.join("benchmarks").join("local-data");
        Self {
            runs_jsonl: local_data.join("runs.jsonl"),
            system_toml: local_data.join("system.toml"),
            summaries: repository_root.join("benchmarks").join("summaries"),
            profile_history: local_data.join("profile-runs.jsonl"),
            profiles: local_data.join("profiles"),
        }
    }
}

/// One prepared benchmark run.
///
/// Owns the preparation facts shared by every mode: the typed manifest, the
/// captured repository snapshot, the computed fingerprints and the anchored
/// local-data paths. Compiler construction, case selection, system identity
/// and persistence stay outside this type.
#[derive(Debug)]
pub(crate) struct PreparedBenchmarkRun {
    pub(crate) manifest: BenchmarkManifest,
    pub(crate) snapshot: BenchmarkRepositorySnapshot,
    pub(crate) fingerprints: BenchmarkFingerprints,
    pub(crate) paths: BenchmarkPaths,
}

impl PreparedBenchmarkRun {
    /// Load the manifest, capture the repository snapshot, apply the recording
    /// eligibility gate, then compute fingerprints. The clean-start rejection
    /// must precede fingerprint traversal and compiler construction.
    pub(crate) fn load(recording: BenchmarkRecording) -> Result<Self, String> {
        let current_directory = std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?;
        Self::load_from(recording, &current_directory)
    }

    /// Load one prepared run from an explicit current directory for tests and
    /// repository-rooted commands.
    pub(crate) fn load_from(
        recording: BenchmarkRecording,
        current_directory: &Path,
    ) -> Result<Self, String> {
        let manifest =
            load_benchmark_manifest_from(current_directory).map_err(|error| error.to_string())?;
        let snapshot = BenchmarkRepositorySnapshot::capture(&manifest.repository_root)
            .map_err(|error| error.to_string())?;

        require_clean_for_recording(recording, &snapshot).map_err(|error| error.to_string())?;

        let fingerprints =
            compute_benchmark_fingerprints(&manifest).map_err(|error| error.to_string())?;
        let paths = BenchmarkPaths::for_repository(&manifest.repository_root);

        Ok(Self {
            manifest,
            snapshot,
            fingerprints,
            paths,
        })
    }

    /// Borrow the repository-anchored local-data paths for this run.
    pub(crate) fn paths(&self) -> &BenchmarkPaths {
        &self.paths
    }

    /// Verify the repository still matches the preparation snapshot.
    pub(crate) fn verify_unchanged(&self) -> Result<(), String> {
        self.snapshot
            .verify_unchanged(&self.manifest.repository_root)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "benchmark_run/tests.rs"]
mod tests;
