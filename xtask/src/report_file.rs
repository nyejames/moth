//! Atomic machine-readable report writes and run identity.
//!
//! WHAT: writes a report to a sibling temporary file, flushes it, then renames it over the final
//!       path, and captures the identity of the run that produced it.
//! WHY:  a report written directly to its final path can be left half-written by an interrupted
//!       run, and a report with no run identity cannot be told apart from a stale one that a
//!       previous, differently configured run left behind. Both make a report that reviewers
//!       treat as evidence into a file that only looks like evidence.
//!
//! A report owner writes its report twice: once with `completed: false` before the work starts,
//! and once with the finished contents. That ordering is what stops an interrupted run from
//! leaving a previous successful report in place where a reader would read it as current.

use serde::Serialize;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A process-local counter that makes two identities from one process distinct.
///
/// Uniqueness must not depend on the wall clock advancing between two captures: two calls in the
/// same nanosecond are a real possibility, and a test that only passes when the clock ticks is a
/// timing test wearing an identity test's name.
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Identity of the run that produced a report.
///
/// WHAT: names the process, command, host, build configuration and completion state behind one
///       report file.
/// WHY:  a report with no run identity cannot be told apart from a stale one a previous,
///       differently configured run left behind, and a report with no completion state cannot be
///       told apart from one an interrupted run abandoned halfway.
///
/// `id` exists to tell two runs apart, not to order them: it pairs the process id with a
/// process-local sequence number, plus wall-clock nanoseconds as descriptive data only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportRunIdentity {
    pub id: String,
    pub command: String,
    pub os: String,
    pub arch: String,
    /// Features the `moth` library linked into this `xtask` binary was built with.
    ///
    /// `xtask` declares no features of its own, so this is the build configuration of the
    /// compiler code any report here describes or measures.
    pub features: Vec<String>,
    /// Worker thread count when the command owns one, `None` when the command is single-threaded
    /// or delegates parallelism to the tools it launches.
    pub thread_count: Option<usize>,
    /// Whether the run that owns this report reached the end of the work the report describes.
    ///
    /// A report is written with `false` before its owned work starts, so an interrupted run
    /// leaves a report that says so rather than a previous successful one that looks current.
    pub completed: bool,
}

impl ReportRunIdentity {
    /// Capture the identity of a run of `command` that has started but not finished.
    pub fn started(command: &str, thread_count: Option<usize>) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);

        Self {
            id: format!("{:x}-{sequence:x}-{nanos:x}", process::id()),
            command: command.to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            features: moth::ENABLED_FEATURES
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            thread_count,
            completed: false,
        }
    }

    /// The same identity, marked as describing finished work.
    pub fn completed(&self) -> Self {
        Self {
            completed: true,
            ..self.clone()
        }
    }
}

/// Write `bytes` to `path` so a reader never observes a partial file.
///
/// The temporary file is a sibling of the final path so the rename stays within one filesystem;
/// a rename across filesystems is a copy, which is exactly the non-atomic write this avoids. A
/// failure after the temporary file exists removes it, so an interrupted write leaves the previous
/// report in place rather than a partial new one beside it.
pub fn write_report_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("report path '{}' has no parent directory", path.display()))?;

    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create '{}': {error}", parent.display()))?;

    let file_name = path
        .file_name()
        .ok_or_else(|| format!("report path '{}' has no file name", path.display()))?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(format!(".{}.partial", process::id()));
    let temporary_path = parent.join(temporary_name);

    if let Err(error) = write_and_sync(&temporary_path, bytes) {
        remove_partial(&temporary_path);
        return Err(error);
    }

    fs::rename(&temporary_path, path).map_err(|error| {
        remove_partial(&temporary_path);
        format!(
            "failed to move '{}' onto '{}': {error}",
            temporary_path.display(),
            path.display()
        )
    })
}

/// Write the complete report body and flush it to the filesystem before it is renamed.
fn write_and_sync(temporary_path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = File::create(temporary_path)
        .map_err(|error| format!("failed to create '{}': {error}", temporary_path.display()))?;

    file.write_all(bytes)
        .map_err(|error| format!("failed to write '{}': {error}", temporary_path.display()))?;

    file.sync_all()
        .map_err(|error| format!("failed to flush '{}': {error}", temporary_path.display()))
}

/// Remove a temporary file after a failed write.
///
/// A removal failure is reported rather than discarded, but it never replaces the write failure
/// that caused it: the caller is already returning the real reason.
fn remove_partial(temporary_path: &Path) {
    if let Err(error) = fs::remove_file(temporary_path) {
        eprintln!(
            "warning: failed to remove the partial report '{}': {error}",
            temporary_path.display()
        );
    }
}

#[cfg(test)]
mod tests;
