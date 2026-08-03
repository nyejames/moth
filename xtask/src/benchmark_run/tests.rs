//! Focused contracts for one prepared benchmark run.

use super::*;
use crate::benchmark_manifest::{
    BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode, BenchmarkRunner,
    BenchmarkWorkload, CliBenchmarkCommand,
};
use crate::benchmark_repository::BenchmarkRepositorySnapshot;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn init_git_repo() -> TempDir {
    let directory = tempfile::tempdir().expect("temporary repository should exist");

    run_git_in(directory.path(), &["init"]);
    run_git_in(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    run_git_in(directory.path(), &["config", "user.name", "Test"]);

    directory
}

fn run_git_in(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git command should succeed");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_file(root: &Path, path: &str, contents: &str) {
    let full_path = root.join(path);
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).expect("parent should be creatable");
    }
    fs::write(full_path, contents).expect("file should be writable");
}

fn commit_all(root: &Path, message: &str) {
    run_git_in(root, &["add", "-A"]);
    run_git_in(root, &["commit", "-m", message]);
}

fn fixture_manifest(repository_root: &Path) -> BenchmarkManifest {
    let workload = BenchmarkWorkload {
        id: "fixture".to_string(),
        entry: PathBuf::from("fixture.moth"),
        entry_kind: BenchmarkEntryKind::File,
        fingerprint_mode: BenchmarkFingerprintMode::FullTree,
        fingerprint_roots: vec![PathBuf::from("fixture.moth")],
        fingerprint_excludes: Vec::new(),
    };
    let case = crate::benchmark_manifest::BenchmarkCase {
        id: "fixture_check".to_string(),
        case_index: 0,
        workload_index: 0,
        group_name: "core".to_string(),
        quick: true,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
    };

    BenchmarkManifest {
        workloads: vec![workload],
        cases: vec![case],
        manifest_path: repository_root.join("benchmarks/manifest.toml"),
        repository_root: repository_root.to_owned(),
    }
}

fn prepared_from_repository(repository_root: &Path) -> PreparedBenchmarkRun {
    let manifest = fixture_manifest(repository_root);
    let snapshot =
        BenchmarkRepositorySnapshot::capture(repository_root).expect("snapshot should capture");
    let fingerprints = crate::benchmark_fingerprint::compute_benchmark_fingerprints(&manifest)
        .expect("fingerprints should compute");

    PreparedBenchmarkRun {
        manifest,
        snapshot,
        fingerprints,
    }
}

#[test]
fn recording_eligibility_applies_only_to_record_mode() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let prepared = prepared_from_repository(repo.path());
    prepared
        .require_recording_eligible(BenchmarkRecording::Record)
        .expect("clean committed preparation should be recording eligible");
    prepared
        .require_recording_eligible(BenchmarkRecording::ReadOnly)
        .expect("read-only mode should pass the eligibility gate");

    write_file(repo.path(), "fixture.moth", "value = 2\n");
    let dirty_prepared = prepared_from_repository(repo.path());
    dirty_prepared
        .require_recording_eligible(BenchmarkRecording::Record)
        .expect_err("dirty preparation must be rejected for recording");
    dirty_prepared
        .require_recording_eligible(BenchmarkRecording::ReadOnly)
        .expect("read-only mode must permit dirty preparation");
}

#[test]
fn prepared_run_verification_detects_source_mutation_after_preparation() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let prepared = prepared_from_repository(repo.path());
    prepared
        .verify_unchanged()
        .expect("unchanged repository should verify");

    write_file(repo.path(), "fixture.moth", "value = 2\n");

    let error = prepared
        .verify_unchanged()
        .expect_err("a source mutation after preparation must be detected");
    assert!(error.contains("changed during benchmark run"));
}
