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
//! Directory-entry `build` cases register their manifest-declared generated
//! output roots with this workspace. `finish()` removes only registered
//! run-owned roots, detects undeclared `.moth_manifest` files, and must
//! succeed before repository verification or persistence. Drop remains a
//! best-effort emergency cleanup and can never define success.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkManifest, BenchmarkManifestError, BenchmarkRunner,
    CliBenchmarkCommand, CliBenchmarkInvocation,
};

/// One run-owned generated output root registered for explicit cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredOutputRoot {
    entry_path: PathBuf,
    root_path: PathBuf,
}

/// Contextual failures while finalising benchmark-generated outputs.
#[derive(Debug)]
pub(crate) enum BenchmarkWorkspaceError {
    /// A declared root already exists before the first execution.
    ExistingOutputRoot { path: PathBuf },
    /// A declared root is tracked by Git and must never be deleted.
    TrackedOutputRoot { path: PathBuf },
    /// A registered root escaped its workload entry.
    RootOutsideEntry { root: PathBuf, entry: PathBuf },
    /// A registered root was replaced by a symlink during the run.
    SymlinkReplacedRoot { root: PathBuf },
    /// Removing a registered root failed.
    RemovalFailed {
        root: PathBuf,
        source: std::io::Error,
    },
    /// A registered root still exists after removal.
    RootStillPresent { root: PathBuf },
    /// A build produced an undeclared `.moth_manifest` outside declared roots.
    UndeclaredManifest { path: PathBuf },
}

impl Display for BenchmarkWorkspaceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExistingOutputRoot { path } => write!(
                formatter,
                "generated output root '{}' must not exist before the run",
                path.display()
            ),
            Self::TrackedOutputRoot { path } => write!(
                formatter,
                "generated output root '{}' is tracked by Git and will never be deleted",
                path.display()
            ),
            Self::RootOutsideEntry { root, entry } => write!(
                formatter,
                "generated output root '{}' escaped its workload entry '{}'",
                root.display(),
                entry.display()
            ),
            Self::SymlinkReplacedRoot { root } => write!(
                formatter,
                "generated output root '{}' was replaced by a symlink during the run",
                root.display()
            ),
            Self::RemovalFailed { root, source } => write!(
                formatter,
                "failed to remove generated output root '{}': {source}",
                root.display()
            ),
            Self::RootStillPresent { root } => write!(
                formatter,
                "generated output root '{}' still exists after removal",
                root.display()
            ),
            Self::UndeclaredManifest { path } => write!(
                formatter,
                "build produced undeclared output manifest '{}' outside declared generated roots",
                path.display()
            ),
        }
    }
}

impl std::error::Error for BenchmarkWorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RemovalFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// One run-scoped ignored workspace under `target/benchmark-work/`.
///
/// The workspace owns the temporary run root and resolves per-case CLI
/// invocations. File-entry cases receive one stable subdirectory that persists
/// across preflight, warmup, measured iterations, observation and Samply.
/// Directory-entry cases use the repository root as their current directory.
pub(crate) struct BenchmarkExecutionWorkspace {
    run_root: tempfile::TempDir,
    repository_root: PathBuf,
    /// Declared generated output roots registered before their first execution.
    registered_roots: std::cell::RefCell<Vec<RegisteredOutputRoot>>,
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
            repository_root: repository_root.to_owned(),
            registered_roots: std::cell::RefCell::new(Vec::new()),
        })
    }

    /// Resolve one CLI invocation from manifest facts for a file-entry or
    /// directory-entry case.
    ///
    /// File-entry cases run from an isolated case directory below the run root.
    /// Directory-entry cases run from the repository root because their output
    /// folders are project-owned and excluded from workload fingerprints.
    ///
    /// For directory-entry `build` cases, the manifest-declared generated
    /// output roots are validated as absent and registered with this workspace
    /// before the first execution. `check` cases and frontend cases register
    /// nothing.
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
                if *command == CliBenchmarkCommand::Build {
                    let entry_path = manifest.repository_root.join(&workload.entry);
                    self.register_generated_roots(
                        manifest,
                        case,
                        &entry_path,
                        &workload.generated_output_roots,
                    )?;
                }

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

    /// Validate and register the declared generated output roots for one
    /// directory-entry `build` case before its first execution.
    ///
    /// The check runs once per entry; later iterations reuse the registration.
    /// Tracked or pre-existing roots are rejected so the run never deletes
    /// user data.
    fn register_generated_roots(
        &self,
        manifest: &BenchmarkManifest,
        case: &BenchmarkCase,
        entry_path: &Path,
        generated_output_roots: &[PathBuf],
    ) -> Result<(), BenchmarkManifestError> {
        let already_registered = self
            .registered_roots
            .borrow()
            .iter()
            .any(|root| root.entry_path == entry_path);
        if already_registered {
            return Ok(());
        }

        let mut roots = Vec::with_capacity(generated_output_roots.len());
        for root in generated_output_roots {
            let root_path = entry_path.join(root);
            if is_git_tracked(&self.repository_root, &root_path) {
                return Err(manifest.runtime_error(
                    case,
                    &BenchmarkWorkspaceError::TrackedOutputRoot { path: root_path }.to_string(),
                ));
            }
            if root_path.exists() || root_path.is_symlink() {
                return Err(manifest.runtime_error(
                    case,
                    &BenchmarkWorkspaceError::ExistingOutputRoot { path: root_path }.to_string(),
                ));
            }
            roots.push(RegisteredOutputRoot {
                entry_path: entry_path.to_owned(),
                root_path,
            });
        }

        self.registered_roots.borrow_mut().extend(roots);
        Ok(())
    }

    /// Cheap per-execution check after a successful directory-entry build.
    ///
    /// Verifies that the run did not create an undeclared `.moth_manifest`
    /// directly under the workload entry. The bounded recursive scan for
    /// deeper drift runs once per affected workload inside `finish()`.
    pub(crate) fn check_directory_build_output(
        &self,
        entry_path: &Path,
    ) -> Result<(), BenchmarkWorkspaceError> {
        if entry_path.join(".moth_manifest").exists() {
            return Err(BenchmarkWorkspaceError::UndeclaredManifest {
                path: entry_path.join(".moth_manifest"),
            });
        }
        Ok(())
    }

    /// Explicitly finalise the run: remove only registered run-owned roots and
    /// detect undeclared output manifests.
    ///
    /// Must succeed before repository verification or persistence. Drop is
    /// best-effort emergency cleanup and never defines success. Idempotent:
    /// already-removed roots are skipped.
    pub(crate) fn finish(&self) -> Result<(), BenchmarkWorkspaceError> {
        let roots = self.registered_roots.borrow().clone();

        for root in &roots {
            if !root.root_path.starts_with(&root.entry_path) {
                return Err(BenchmarkWorkspaceError::RootOutsideEntry {
                    root: root.root_path.clone(),
                    entry: root.entry_path.clone(),
                });
            }
            if root.root_path.is_symlink() {
                return Err(BenchmarkWorkspaceError::SymlinkReplacedRoot {
                    root: root.root_path.clone(),
                });
            }
            if !root.root_path.exists() {
                continue;
            }

            std::fs::remove_dir_all(&root.root_path).map_err(|source| {
                BenchmarkWorkspaceError::RemovalFailed {
                    root: root.root_path.clone(),
                    source,
                }
            })?;

            if root.root_path.exists() {
                return Err(BenchmarkWorkspaceError::RootStillPresent {
                    root: root.root_path.clone(),
                });
            }
        }

        let mut scanned_entries = Vec::new();
        for root in &roots {
            if scanned_entries.contains(&root.entry_path) {
                continue;
            }
            scanned_entries.push(root.entry_path.clone());
            self.scan_for_undeclared_manifests(&root.entry_path, &roots)?;
        }

        Ok(())
    }

    /// One bounded recursive scan per affected workload for `.moth_manifest`
    /// files outside the declared generated roots.
    fn scan_for_undeclared_manifests(
        &self,
        entry_path: &Path,
        roots: &[RegisteredOutputRoot],
    ) -> Result<(), BenchmarkWorkspaceError> {
        let mut pending = vec![entry_path.to_owned()];
        while let Some(directory) = pending.pop() {
            let entries = std::fs::read_dir(&directory).map_err(|source| {
                BenchmarkWorkspaceError::RemovalFailed {
                    root: directory.clone(),
                    source,
                }
            })?;

            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = entry.file_name();

                if file_name == ".moth_manifest" {
                    return Err(BenchmarkWorkspaceError::UndeclaredManifest { path });
                }

                if path.is_dir() {
                    let inside_declared_root = roots
                        .iter()
                        .filter(|root| root.entry_path == entry_path)
                        .any(|root| root.root_path == path);
                    if !inside_declared_root {
                        pending.push(path);
                    }
                }
            }
        }

        Ok(())
    }
}

/// Explicitly finalise run-owned outputs and combine operation and cleanup
/// failures.
///
/// Runs before repository verification and persistence in every suite and
/// profile flow. When the operation and cleanup both fail, both causes are
/// reported.
pub(crate) fn finalise_workspace(
    workspace: &BenchmarkExecutionWorkspace,
    result: Result<(), String>,
) -> Result<(), String> {
    match (result, workspace.finish()) {
        (Err(operation), Err(cleanup_error)) => Err(format!(
            "{operation}\nworkspace cleanup also failed: {cleanup_error}"
        )),
        (Err(operation), Ok(())) => Err(operation),
        (Ok(()), Err(cleanup_error)) => Err(format!("workspace cleanup failed: {cleanup_error}")),
        (Ok(()), Ok(())) => Ok(()),
    }
}

impl Drop for BenchmarkExecutionWorkspace {
    fn drop(&mut self) {
        // Best-effort emergency cleanup only. A successful run must call
        // `finish()` explicitly; this path can never define success.
        let roots = self.registered_roots.borrow().clone();
        for root in roots {
            if root.root_path.starts_with(&root.entry_path)
                && !root.root_path.is_symlink()
                && root.root_path.exists()
            {
                let _ = std::fs::remove_dir_all(&root.root_path);
            }
        }
    }
}

/// Check whether a path is tracked by Git in the canonical repository.
///
/// Uses `git ls-files --error-unmatch` to determine if the path is tracked.
/// Returns `false` when the path is not tracked or Git is unavailable.
fn is_git_tracked(repository_root: &Path, path: &Path) -> bool {
    let output = std::process::Command::new("git")
        .current_dir(repository_root)
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
