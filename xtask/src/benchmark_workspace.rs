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
//!
//! Directory-entry cases leave compiler output directories (`dev/` and
//! `release/`) under their entry paths. The workspace tracks these and
//! removes them on drop so benchmark runs do not pollute the repository with
//! generated artifacts.

use std::path::{Path, PathBuf};

use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkManifest, BenchmarkManifestError, BenchmarkRunner,
    CliBenchmarkInvocation,
};

/// Compiler output directories created by `mo build` under a directory entry.
const COMPILER_OUTPUT_DIRS: &[&str] = &["dev", "release"];

/// One run-scoped ignored workspace under `target/benchmark-work/`.
///
/// The workspace owns the temporary run root and resolves per-case CLI
/// invocations. File-entry cases receive one stable subdirectory that persists
/// across preflight, warmup, measured iterations, observation and Samply.
/// Directory-entry cases use the repository root as their current directory.
/// Compiler output directories left by directory-entry builds are cleaned up
/// on drop.
pub(crate) struct BenchmarkExecutionWorkspace {
    run_root: tempfile::TempDir,
    /// Directory-entry paths whose compiler output (`dev/`, `release/`) should
    /// be cleaned on drop. Registered before execution so artifacts created
    /// during the run are removed even if the run fails.
    artifact_entry_paths: std::cell::RefCell<Vec<PathBuf>>,
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

        Ok(Self {
            run_root,
            artifact_entry_paths: std::cell::RefCell::new(Vec::new()),
        })
    }

    /// Resolve one CLI invocation from manifest facts for a file-entry or
    /// directory-entry case.
    ///
    /// File-entry cases run from an isolated case directory below the run root.
    /// Directory-entry cases run from the repository root because their output
    /// folders are project-owned and excluded from workload fingerprints.
    ///
    /// For directory-entry cases, the compiler output directories (`dev/` and
    /// `release/`) under the entry path are registered for cleanup on drop so
    /// generated artifacts do not persist after the run.
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
            BenchmarkEntryKind::Directory => {
                let entry_path = manifest.repository_root.join(&workload.entry);

                self.register_directory_artifacts(&entry_path);

                (
                    manifest.repository_root.clone(),
                    workload.entry.display().to_string(),
                )
            }
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

    /// Register a directory-entry path for compiler output cleanup on drop.
    ///
    /// Called by `resolve_cli_invocation` for CLI cases and by the frontend
    /// execution path for frontend cases. The entry path is registered before
    /// the compiler runs so artifacts created during the run are removed on
    /// drop even if the run fails. On drop, `dev/` and `release/` under the
    /// entry path are removed only if they are not tracked by Git.
    pub(crate) fn register_directory_artifacts(&self, entry_path: &Path) {
        // Avoid duplicate registration for the same entry path.
        let existing = self.artifact_entry_paths.borrow();
        if existing.iter().any(|path| path == entry_path) {
            return;
        }
        drop(existing);
        self.artifact_entry_paths
            .borrow_mut()
            .push(entry_path.to_path_buf());
    }
}

impl Drop for BenchmarkExecutionWorkspace {
    fn drop(&mut self) {
        // TempDir handles the run root. Clean up compiler output directories
        // left by directory-entry builds so the repository stays clean.
        let entry_paths = self
            .artifact_entry_paths
            .borrow_mut()
            .drain(..)
            .collect::<Vec<_>>();
        for entry_path in entry_paths {
            for output_dir in COMPILER_OUTPUT_DIRS {
                let artifact_path = entry_path.join(output_dir);
                if artifact_path.is_dir() && !is_git_tracked(&artifact_path) {
                    let _ = std::fs::remove_dir_all(&artifact_path);
                }
            }
        }
    }
}

/// Check whether a path is tracked by Git.
///
/// Uses `git ls-files --error-unmatch` to determine if the path is tracked.
/// Returns `false` when the path is not tracked or Git is unavailable.
fn is_git_tracked(path: &Path) -> bool {
    let output = std::process::Command::new("git")
        .arg("ls-files")
        .arg("--error-unmatch")
        .arg(path)
        .output();

    match output {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "benchmark_workspace/tests.rs"]
mod tests;
