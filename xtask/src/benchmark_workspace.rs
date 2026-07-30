//! Run-scoped workspace for isolated benchmark execution.
//!
//! WHAT: Creates one ignored temporary directory under `target/benchmark-work/`
//! and resolves one CLI invocation from manifest facts for each case.
//! WHY: File-entry CLI cases must run from an isolated directory so compiler
//! output never writes into the tracked checkout. Directory-entry cases keep
//! the repository root because their output folders are project-owned, ignored
//! and excluded from workload fingerprints. One owner resolves the invocation
//! so preflight, measured iterations, observation profiling and Samply all
//! consume the same command, arguments and working directory.

use std::path::Path;

use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkManifest, BenchmarkManifestError, BenchmarkRunner,
    CliBenchmarkInvocation,
};

/// One run-scoped ignored workspace under `target/benchmark-work/`.
///
/// The workspace owns the temporary run root and resolves per-case CLI
/// invocations. File-entry cases receive one stable subdirectory that persists
/// across preflight, warmup, measured iterations, observation and Samply.
/// Directory-entry cases use the repository root as their current directory.
pub(crate) struct BenchmarkExecutionWorkspace {
    run_root: tempfile::TempDir,
}

impl std::fmt::Debug for BenchmarkExecutionWorkspace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BenchmarkExecutionWorkspace")
            .field("run_root", &self.run_root.path())
            .finish()
    }
}

impl BenchmarkExecutionWorkspace {
    /// Create one unique run directory below `target/benchmark-work/` inside the
    /// canonical repository root.
    ///
    /// The directory is created under `repository_root/target/benchmark-work/`
    /// and cleaned up when the workspace is dropped.
    pub(crate) fn create(repository_root: &Path) -> Result<Self, String> {
        let workspace_root = repository_root.join("target").join("benchmark-work");
        std::fs::create_dir_all(&workspace_root).map_err(|error| {
            format!(
                "failed to create benchmark workspace directory '{}': {error}",
                workspace_root.display()
            )
        })?;

        let run_root = tempfile::TempDir::new_in(&workspace_root).map_err(|error| {
            format!(
                "failed to create benchmark run directory under '{}': {error}",
                workspace_root.display()
            )
        })?;

        Ok(Self { run_root })
    }

    /// Resolve one CLI invocation from manifest facts for a file-entry or
    /// directory-entry case.
    ///
    /// File-entry cases run from an isolated case directory below the run root.
    /// Directory-entry cases run from the repository root because their output
    /// folders are project-owned and excluded from workload fingerprints.
    ///
    /// The case ID already uses a restricted safe alphabet, so it is used
    /// directly for the case directory name without a second sanitiser.
    pub(crate) fn resolve_cli_invocation(
        &self,
        manifest: &BenchmarkManifest,
        case: &BenchmarkCase,
    ) -> Result<CliBenchmarkInvocation, BenchmarkManifestError> {
        let workload = manifest
            .workload_for(case)
            .ok_or_else(|| manifest.runtime_error(case, "workload relationship is invalid"))?;
        let BenchmarkRunner::Cli { command, args } = &case.runner else {
            return Err(manifest.runtime_error(case, "case does not declare a CLI runner"));
        };

        let (current_directory, entry_argument) = match workload.entry_kind {
            BenchmarkEntryKind::File => {
                let case_directory = self.run_root.path().join(&case.id);
                std::fs::create_dir_all(&case_directory).map_err(|error| {
                    BenchmarkManifestError::Invalid {
                        path: manifest.manifest_path.clone(),
                        subject: format!("case '{}'", case.id),
                        message: format!(
                            "failed to create isolated case directory '{}': {error}",
                            case_directory.display()
                        ),
                    }
                })?;

                // Use the absolute entry path so the current directory change
                // cannot alter source resolution. The entry was already
                // validated against the repository root during manifest load.
                let absolute_entry = manifest.repository_root.join(&workload.entry);

                (case_directory, absolute_entry.display().to_string())
            }
            BenchmarkEntryKind::Directory => (
                manifest.repository_root.clone(),
                workload.entry.display().to_string(),
            ),
        };

        let mut invocation_args = Vec::with_capacity(args.len() + 1);
        invocation_args.push(entry_argument);
        invocation_args.extend(args.iter().cloned());

        Ok(CliBenchmarkInvocation {
            command: *command,
            args: invocation_args,
            current_directory,
        })
    }
}

#[cfg(test)]
#[path = "benchmark_workspace/tests.rs"]
mod tests;
