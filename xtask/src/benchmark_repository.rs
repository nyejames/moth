//! Repository state capture and mutation verification for benchmark runs.
//!
//! WHAT: Captures the complete Git source state before a benchmark run and
//! verifies it remains unchanged afterwards.
//! WHY: A clean benchmark run must not persist history or summaries when the
//! repository changed during measurement. Harness-created files, source edits
//! during a run, or commit changes must all be detected. Porcelain status
//! alone cannot detect a second edit to a file that started dirty, so the
//! snapshot retains the full tracked diff bytes and untracked content
//! identities.

use crate::bench_types::GitRevision;
use std::path::Path;
use std::process::Command;

/// One captured repository state at the start of a benchmark run.
///
/// Retains enough data to detect any tracked or untracked change, including a
/// second edit to a file that started dirty. Porcelain status is kept for
/// readable diagnostics but is not the comparison authority.
#[derive(Debug, Clone)]
pub(crate) struct BenchmarkRepositorySnapshot {
    commit: String,
    tracked_diff: Vec<u8>,
    untracked_files: Vec<UntrackedFileSnapshot>,
    porcelain_status: Vec<u8>,
}

/// One untracked file with its path and content identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UntrackedFileSnapshot {
    path: String,
    content_hash: String,
}

/// Contextual failures while capturing or verifying repository state.
#[derive(Debug)]
pub(crate) enum BenchmarkRepositoryError {
    GitCommand {
        command: String,
        source: std::io::Error,
    },
    GitOutput {
        command: String,
        stderr: String,
    },
    InvalidUtf8 {
        command: String,
    },
}

impl std::fmt::Display for BenchmarkRepositoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitCommand { command, source } => {
                write!(formatter, "git command '{command}' failed: {source}")
            }
            Self::GitOutput { command, stderr } => {
                write!(
                    formatter,
                    "git command '{command}' produced an error: {stderr}"
                )
            }
            Self::InvalidUtf8 { command } => {
                write!(
                    formatter,
                    "git command '{command}' produced non-UTF-8 output"
                )
            }
        }
    }
}

impl std::error::Error for BenchmarkRepositoryError {}

impl BenchmarkRepositorySnapshot {
    /// Capture the complete repository state from the canonical repository root.
    ///
    /// Runs four Git commands to record the commit, tracked diff, untracked
    /// files with content identities, and porcelain status.
    pub(crate) fn capture(repository_root: &Path) -> Result<Self, BenchmarkRepositoryError> {
        let commit = capture_commit(repository_root)?;
        let tracked_diff = capture_tracked_diff(repository_root)?;
        let untracked_files = capture_untracked_files(repository_root)?;
        let porcelain_status = capture_porcelain_status(repository_root)?;

        Ok(Self {
            commit,
            tracked_diff,
            untracked_files,
            porcelain_status,
        })
    }

    /// Produce the `GitRevision` persisted with a run.
    ///
    /// The dirty flag is derived from whether the tracked diff or untracked
    /// file set is non-empty.
    pub(crate) fn git_revision(&self) -> GitRevision {
        GitRevision {
            commit: Some(self.commit.clone()),
            dirty: Some(!self.tracked_diff.is_empty() || !self.untracked_files.is_empty()),
        }
    }

    /// Verify that the repository state matches this snapshot.
    ///
    /// Compares the commit, tracked diff bytes and untracked content
    /// identities. A file that started dirty and changed again is rejected
    /// even when its porcelain code stays the same.
    pub(crate) fn verify_unchanged(
        &self,
        repository_root: &Path,
    ) -> Result<(), BenchmarkRepositoryError> {
        let current_commit = capture_commit(repository_root)?;
        if current_commit != self.commit {
            return Err(BenchmarkRepositoryError::GitOutput {
                command: "rev-parse --verify HEAD".to_string(),
                stderr: format!(
                    "commit changed during benchmark run: was '{}', now '{}'",
                    self.commit, current_commit
                ),
            });
        }

        let current_diff = capture_tracked_diff(repository_root)?;
        if current_diff != self.tracked_diff {
            let changed_entries = format_changed_porcelain_entries(repository_root);
            return Err(BenchmarkRepositoryError::GitOutput {
                command: "diff --binary --full-index --no-ext-diff HEAD".to_string(),
                stderr: format!(
                    "tracked files changed during benchmark run{}",
                    changed_entries
                ),
            });
        }

        let current_untracked = capture_untracked_files(repository_root)?;
        if current_untracked != self.untracked_files {
            let changed_entries = format_changed_porcelain_entries(repository_root);
            return Err(BenchmarkRepositoryError::GitOutput {
                command: "ls-files --others --exclude-standard".to_string(),
                stderr: format!(
                    "untracked files changed during benchmark run{}",
                    changed_entries
                ),
            });
        }

        Ok(())
    }
}

fn capture_commit(repository_root: &Path) -> Result<String, BenchmarkRepositoryError> {
    let output = run_git(
        repository_root,
        &["rev-parse", "--verify", "HEAD"],
        "rev-parse --verify HEAD",
    )?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| BenchmarkRepositoryError::InvalidUtf8 {
            command: "rev-parse --verify HEAD".to_string(),
        })
}

fn capture_tracked_diff(repository_root: &Path) -> Result<Vec<u8>, BenchmarkRepositoryError> {
    let output = run_git(
        repository_root,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "HEAD",
            "--",
        ],
        "diff --binary --full-index --no-ext-diff HEAD",
    )?;
    Ok(output.stdout)
}

fn capture_porcelain_status(repository_root: &Path) -> Result<Vec<u8>, BenchmarkRepositoryError> {
    let output = run_git(
        repository_root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        "status --porcelain=v1 -z",
    )?;
    Ok(output.stdout)
}

fn capture_untracked_files(
    repository_root: &Path,
) -> Result<Vec<UntrackedFileSnapshot>, BenchmarkRepositoryError> {
    let output = run_git(
        repository_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        "ls-files --others --exclude-standard -z",
    )?;

    let paths = parse_nul_delimited(&output.stdout);
    let mut snapshots = Vec::with_capacity(paths.len());

    for path in paths {
        if path.is_empty() {
            continue;
        }

        let hash = capture_file_hash(repository_root, &path)?;
        snapshots.push(UntrackedFileSnapshot {
            path: path.to_owned(),
            content_hash: hash,
        });
    }

    snapshots.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(snapshots)
}

fn capture_file_hash(
    repository_root: &Path,
    path: &str,
) -> Result<String, BenchmarkRepositoryError> {
    let command_label = format!("hash-object --no-filters -- {path}");
    let output = run_git(
        repository_root,
        &["hash-object", "--no-filters", "--", path],
        &command_label,
    )?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| BenchmarkRepositoryError::InvalidUtf8 {
            command: command_label,
        })
}

fn run_git(
    repository_root: &Path,
    args: &[&str],
    command_label: &str,
) -> Result<std::process::Output, BenchmarkRepositoryError> {
    let output = Command::new("git")
        .current_dir(repository_root)
        .args(args)
        .output()
        .map_err(|source| BenchmarkRepositoryError::GitCommand {
            command: command_label.to_string(),
            source,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(BenchmarkRepositoryError::GitOutput {
            command: command_label.to_string(),
            stderr,
        });
    }

    Ok(output)
}

fn parse_nul_delimited(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .split('\0')
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_owned())
        .collect()
}

fn format_changed_porcelain_entries(repository_root: &Path) -> String {
    match capture_porcelain_status(repository_root) {
        Ok(status) => {
            let entries = String::from_utf8_lossy(&status);
            let readable: Vec<&str> = entries.split('\0').filter(|s| !s.is_empty()).collect();
            if readable.is_empty() {
                String::new()
            } else {
                format!(":\n  {}", readable.join("\n  "))
            }
        }
        Err(_) => String::new(),
    }
}

/// Combine an operation result with final repository verification.
///
/// Required behaviour:
/// - operation succeeds, repository unchanged -> return the result
/// - operation succeeds, repository changed -> return repository mutation error
/// - operation fails, repository unchanged -> return operation error
/// - operation fails, repository changed -> return an error containing both failures
pub(crate) fn verify_after_operation<T, E: std::fmt::Display>(
    snapshot: &BenchmarkRepositorySnapshot,
    repository_root: &Path,
    operation: Result<T, E>,
) -> Result<T, String> {
    match operation {
        Ok(value) => match snapshot.verify_unchanged(repository_root) {
            Ok(()) => Ok(value),
            Err(error) => Err(error.to_string()),
        },
        Err(operation_error) => match snapshot.verify_unchanged(repository_root) {
            Ok(()) => Err(operation_error.to_string()),
            Err(mutation_error) => Err(format!(
                "benchmark operation failed: {operation_error}\n\
                 repository also changed during the run: {mutation_error}"
            )),
        },
    }
}

#[cfg(test)]
#[path = "benchmark_repository/tests.rs"]
mod tests;
