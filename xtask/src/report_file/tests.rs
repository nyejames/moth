//! Self-tests for atomic report writes and run identity.

use super::{ReportRunIdentity, write_report_atomically};
use crate::test_fs::assert_directory;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

/// File names directly under `directory`, sorted.
///
/// A name that is not UTF-8 fails the read rather than being replaced: these names are the
/// assertion, and a substituted character names a different file.
fn file_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("the directory should be readable")
        .map(|entry| {
            entry
                .expect("the entry should be readable")
                .file_name()
                .to_str()
                .expect("temporary names are UTF-8")
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn writes_the_complete_report_to_the_final_path() {
    let workspace = tempdir().expect("temp dir");
    let report_path = workspace.path().join("reports").join("coverage.json");

    write_report_atomically(&report_path, b"{\"ok\":true}").expect("the report should be written");

    assert_eq!(
        fs::read_to_string(&report_path).expect("the report should be readable"),
        "{\"ok\":true}"
    );
}

#[test]
fn leaves_no_partial_file_beside_a_written_report() {
    let workspace = tempdir().expect("temp dir");
    let report_path = workspace.path().join("coverage.json");

    write_report_atomically(&report_path, b"{}").expect("the report should be written");

    assert_eq!(
        file_names(workspace.path()),
        vec!["coverage.json".to_string()]
    );
}

#[test]
fn replaces_a_previous_report_rather_than_appending_to_it() {
    let workspace = tempdir().expect("temp dir");
    let report_path = workspace.path().join("coverage.json");

    write_report_atomically(&report_path, b"{\"run\":1}").expect("the first write should succeed");
    write_report_atomically(&report_path, b"{\"r\":2}").expect("the second write should succeed");

    assert_eq!(
        fs::read_to_string(&report_path).expect("the report should be readable"),
        "{\"r\":2}"
    );
}

#[test]
fn a_failed_write_leaves_the_previous_report_in_place() {
    let workspace = tempdir().expect("temp dir");
    let report_path = workspace.path().join("coverage.json");
    write_report_atomically(&report_path, b"{\"run\":1}").expect("the first write should succeed");

    // A directory occupying the temporary path blocks the new write without touching the report.
    let blocking_path = workspace
        .path()
        .join(format!("coverage.json.{}.partial", std::process::id()));
    fs::create_dir(&blocking_path).expect("the blocking directory should be created");

    let error = write_report_atomically(&report_path, b"{\"run\":2}")
        .expect_err("a blocked temporary path must fail the write");

    assert!(
        error.contains("failed to create") && error.contains("partial"),
        "unexpected error: {error}"
    );
    assert_eq!(
        fs::read_to_string(&report_path).expect("the previous report should still be readable"),
        "{\"run\":1}"
    );
}

#[test]
fn rejects_a_report_path_that_is_an_existing_directory() {
    let workspace = tempdir().expect("temp dir");
    let report_path = workspace.path().join("coverage.json");
    fs::create_dir(&report_path).expect("the blocking directory should be created");

    let error = write_report_atomically(&report_path, b"{}")
        .expect_err("a directory cannot be replaced by a report");

    assert!(
        error.contains("failed to move"),
        "unexpected error: {error}"
    );
    assert_directory(&report_path);
    assert_eq!(
        file_names(workspace.path()),
        vec!["coverage.json".to_string()],
        "a failed rename must not leave a partial file behind"
    );
}

#[test]
fn rejects_a_report_path_with_no_file_name() {
    let workspace = tempdir().expect("temp dir");
    let report_path = workspace.path().join("reports").join("..");

    let error = write_report_atomically(&report_path, b"{}")
        .expect_err("a path with no file name is not a report path");

    assert!(
        error.contains("has no file name"),
        "unexpected error: {error}"
    );
}

#[test]
fn run_identity_records_the_command_and_the_host() {
    let identity = ReportRunIdentity::started("feature-lane-check", None);

    assert_eq!(identity.command, "feature-lane-check");
    assert_eq!(identity.os, std::env::consts::OS);
    assert_eq!(identity.arch, std::env::consts::ARCH);
}

#[test]
fn run_identity_records_the_build_configuration_of_the_linked_compiler() {
    let identity = ReportRunIdentity::started("source-audit", None);

    let expected: Vec<String> = moth::ENABLED_FEATURES
        .iter()
        .map(|feature| (*feature).to_string())
        .collect();
    assert_eq!(identity.features, expected);
    assert!(
        identity.features.iter().any(|feature| feature == "timers"),
        "xtask depends on moth with features = [\"timers\"], so every xtask report describes a \
         timers build: {:?}",
        identity.features
    );
}

#[test]
fn a_started_run_is_not_completed_and_carries_no_thread_count_by_default() {
    let identity = ReportRunIdentity::started("source-audit", None);

    assert!(!identity.completed);
    assert_eq!(identity.thread_count, None);
}

#[test]
fn a_run_that_owns_a_thread_count_records_it() {
    let identity = ReportRunIdentity::started("stress", Some(16));

    assert_eq!(identity.thread_count, Some(16));
}

#[test]
fn completing_a_run_changes_only_the_completion_state() {
    let started = ReportRunIdentity::started("feature-matrix", Some(4));
    let completed = started.completed();

    assert!(!started.completed);
    assert!(completed.completed);
    assert_eq!(completed.id, started.id);
    assert_eq!(completed.command, started.command);
    assert_eq!(completed.os, started.os);
    assert_eq!(completed.arch, started.arch);
    assert_eq!(completed.features, started.features);
    assert_eq!(completed.thread_count, started.thread_count);
}

#[test]
fn two_runs_do_not_share_one_identity() {
    let first = ReportRunIdentity::started("feature-matrix", None);
    let second = ReportRunIdentity::started("feature-matrix", None);

    assert_ne!(first.id, second.id);
}

/// Uniqueness within one process must come from the sequence, not from the clock advancing.
///
/// The wall clock is only descriptive data in the id. If it were the part that separated two
/// captures, `two_runs_do_not_share_one_identity` would be a clock-resolution test: it would pass
/// on a machine whose clock ticks between two calls and fail on one whose clock does not.
#[test]
fn identity_uniqueness_comes_from_the_process_local_sequence() {
    /// The `sequence` field of a `process-sequence-clock` id.
    fn sequence_of(identity: &ReportRunIdentity) -> &str {
        let mut parts = identity.id.split('-');
        parts.next().expect("an id names its process");
        parts.next().expect("an id names its sequence")
    }

    let first = ReportRunIdentity::started("feature-matrix", None);
    let second = ReportRunIdentity::started("feature-matrix", None);

    assert_ne!(
        sequence_of(&first),
        sequence_of(&second),
        "the process-local sequence must advance between two captures, whatever the clock did"
    );
}
