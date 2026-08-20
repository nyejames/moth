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
    let identity = ReportRunIdentity::capture("feature-lane-check");

    assert_eq!(identity.command, "feature-lane-check");
    assert_eq!(identity.os, std::env::consts::OS);
    assert_eq!(identity.arch, std::env::consts::ARCH);
}

#[test]
fn two_runs_do_not_share_one_identity() {
    let first = ReportRunIdentity::capture("feature-matrix");
    let second = ReportRunIdentity::capture("feature-matrix");

    assert_ne!(first.id, second.id);
}
