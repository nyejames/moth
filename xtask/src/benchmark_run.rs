//! One prepared benchmark run: manifest, repository snapshot and fingerprints.
//!
//! WHAT: Loads the typed manifest, captures the repository snapshot and
//! computes workload and case fingerprints once, in that exact order.
//! WHY: Every benchmark, validation and profile mode must observe one source
//! state before fingerprint traversal, compiler construction, system identity
//! creation or persistence, so recording eligibility and identity facts come
//! from one preparation boundary instead of being reconstructed per command.

use crate::bench_types::BenchmarkRecording;
use crate::benchmark_fingerprint::{BenchmarkFingerprints, compute_benchmark_fingerprints};
use crate::benchmark_manifest::{BenchmarkManifest, load_benchmark_manifest};
use crate::benchmark_repository::{BenchmarkRepositorySnapshot, require_clean_for_recording};

/// One prepared benchmark run.
///
/// Owns the three preparation facts shared by every mode: the typed manifest,
/// the captured repository snapshot and the computed fingerprints. Compiler
/// construction, case selection, system identity and persistence stay outside
/// this type.
#[derive(Debug)]
pub(crate) struct PreparedBenchmarkRun {
    pub(crate) manifest: BenchmarkManifest,
    pub(crate) snapshot: BenchmarkRepositorySnapshot,
    pub(crate) fingerprints: BenchmarkFingerprints,
}

impl PreparedBenchmarkRun {
    /// Load the manifest, capture the repository snapshot, then compute
    /// fingerprints. The snapshot must precede fingerprint traversal.
    pub(crate) fn load() -> Result<Self, String> {
        let manifest = load_benchmark_manifest().map_err(|error| error.to_string())?;
        let snapshot = BenchmarkRepositorySnapshot::capture(&manifest.repository_root)
            .map_err(|error| error.to_string())?;
        let fingerprints =
            compute_benchmark_fingerprints(&manifest).map_err(|error| error.to_string())?;

        Ok(Self {
            manifest,
            snapshot,
            fingerprints,
        })
    }

    /// Apply the clean committed recording gate when the caller records.
    ///
    /// Read-only modes skip the clean-start requirement and keep their final
    /// unchanged verification.
    pub(crate) fn require_recording_eligible(
        &self,
        recording: BenchmarkRecording,
    ) -> Result<(), String> {
        require_clean_for_recording(recording, &self.snapshot).map_err(|error| error.to_string())
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
