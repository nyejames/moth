//! Focused contracts for one prepared benchmark run.

use super::*;
use crate::bench_types::BenchmarkRecording;
use std::fs;
use std::path::Path;
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

fn write_fixture_manifest(repository_root: &Path) {
    let contents = r#"schema = 3

[[workload]]
id = "fixture"
entry = "fixture.moth"
fingerprint_mode = "full_tree"
fingerprint_roots = ["fixture.moth"]
fingerprint_excludes = []
generated_output_roots = []

[[case]]
id = "fixture_check"
workload = "fixture"
group = "core"
quick = true
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
"#;
    write_file(repository_root, "benchmarks/manifest.toml", contents);
}

fn write_invalid_manifest(repository_root: &Path) {
    let contents = r#"schema = 3

[[workload]]
id = "fixture"
entry = "linked.moth"
fingerprint_mode = "full_tree"
fingerprint_roots = ["linked.moth"]
fingerprint_excludes = []
generated_output_roots = []

[[case]]
id = "fixture_check"
workload = "fixture"
group = "core"
quick = true
expectation = "clean"

[case.runner]
kind = "cli"
command = "check"
args = []
"#;
    write_file(repository_root, "benchmarks/manifest.toml", contents);
}

#[test]
fn recording_eligibility_applies_only_to_record_mode() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    write_fixture_manifest(repo.path());
    commit_all(repo.path(), "initial");

    let prepared = PreparedBenchmarkRun::load_from(BenchmarkRecording::Record, repo.path())
        .expect("clean committed preparation should be recording eligible");
    assert!(
        prepared.snapshot.is_clean_committed(),
        "record preparation must capture a clean snapshot"
    );

    PreparedBenchmarkRun::load_from(BenchmarkRecording::ReadOnly, repo.path())
        .expect("read-only mode should pass the eligibility gate");
}

#[test]
#[cfg(unix)]
fn recording_rejects_dirty_repository_before_fingerprint_traversal() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    write_fixture_manifest(repo.path());
    commit_all(repo.path(), "initial");

    // A dirty tracked file plus a symlink fingerprint root that would fail if
    // fingerprint traversal ever ran. Recording must report the clean-worktree
    // failure first, before the fingerprint traversal.
    write_file(repo.path(), "fixture.moth", "value = 2\n");
    std::os::unix::fs::symlink("fixture.moth", repo.path().join("linked.moth"))
        .expect("fingerprint symlink should be creatable");
    write_invalid_manifest(repo.path());

    let error = PreparedBenchmarkRun::load_from(BenchmarkRecording::Record, repo.path())
        .expect_err("dirty preparation must be rejected for recording");
    assert!(
        error.contains("not clean and committed"),
        "record load must fail on the dirty tree before fingerprints, got: {error}"
    );
    assert!(
        !error.contains("is a symlink"),
        "fingerprint traversal must not run before the clean gate, got: {error}"
    );

    let read_only_error =
        PreparedBenchmarkRun::load_from(BenchmarkRecording::ReadOnly, repo.path())
            .expect_err("read-only mode must still compute fingerprints");
    assert!(
        read_only_error.contains("is a symlink"),
        "read-only load should reach the fingerprint failure, got: {read_only_error}"
    );
}

#[test]
fn prepared_run_paths_are_anchored_to_repository_root() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    write_fixture_manifest(repo.path());
    commit_all(repo.path(), "initial");

    let prepared = PreparedBenchmarkRun::load_from(BenchmarkRecording::ReadOnly, repo.path())
        .expect("preparation should succeed");

    let canonical_root = repo
        .path()
        .canonicalize()
        .expect("temporary repository should canonicalize");
    assert_eq!(
        prepared.paths.runs_jsonl,
        canonical_root.join("benchmarks/local-data/runs.jsonl")
    );
    assert_eq!(
        prepared.paths.system_toml,
        canonical_root.join("benchmarks/local-data/system.toml")
    );
    assert_eq!(
        prepared.paths.summaries,
        canonical_root.join("benchmarks/summaries")
    );
    assert_eq!(
        prepared.paths.profile_history,
        canonical_root.join("benchmarks/local-data/profile-runs.jsonl")
    );
    assert_eq!(
        prepared.paths.profiles,
        canonical_root.join("benchmarks/local-data/profiles")
    );
}

#[test]
fn prepared_run_verification_detects_source_mutation_after_preparation() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    write_fixture_manifest(repo.path());
    commit_all(repo.path(), "initial");

    let prepared = PreparedBenchmarkRun::load_from(BenchmarkRecording::ReadOnly, repo.path())
        .expect("preparation should succeed");
    prepared
        .verify_unchanged()
        .expect("unchanged repository should verify");

    write_file(repo.path(), "fixture.moth", "value = 2\n");

    let error = prepared
        .verify_unchanged()
        .expect_err("a source mutation after preparation must be detected");
    assert!(error.contains("changed during benchmark run"));
}
