use super::*;
use std::fs;
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

#[test]
fn clean_committed_recording_eligibility_passes() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    assert!(snapshot.is_clean_committed());
    snapshot
        .require_clean_committed()
        .expect("clean committed state should be eligible for recording");
    require_clean_for_recording(crate::bench_types::BenchmarkRecording::Record, &snapshot)
        .expect("recording gate should pass for a clean committed worktree");
    require_clean_for_recording(crate::bench_types::BenchmarkRecording::ReadOnly, &snapshot)
        .expect("read-only gate should pass for a clean committed worktree");
}

#[test]
fn tracked_dirty_recording_eligibility_fails() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    write_file(repo.path(), "file.moth", "value = 2\n");
    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    assert!(!snapshot.is_clean_committed());
    let error = snapshot
        .require_clean_committed()
        .expect_err("tracked dirty state must not be recording-eligible");
    assert!(error.to_string().contains("uncommitted tracked changes"));
    require_clean_for_recording(crate::bench_types::BenchmarkRecording::Record, &snapshot)
        .expect_err("recording gate must reject a tracked dirty worktree");
}

#[test]
fn untracked_dirty_recording_eligibility_fails() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    write_file(repo.path(), "untracked.moth", "value = 1\n");
    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    assert!(!snapshot.is_clean_committed());
    let error = snapshot
        .require_clean_committed()
        .expect_err("untracked state must not be recording-eligible");
    assert!(error.to_string().contains("untracked files"));
}

#[test]
fn read_only_gate_allows_unchanged_dirty_snapshot() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    write_file(repo.path(), "file.moth", "value = 2\n");
    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    require_clean_for_recording(crate::bench_types::BenchmarkRecording::ReadOnly, &snapshot)
        .expect("read-only gate must permit a dirty but unchanged worktree");
    snapshot
        .verify_unchanged(repo.path())
        .expect("read-only unchanged verification must still pass");
}

#[test]
fn clean_repository_remains_accepted() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    snapshot
        .verify_unchanged(repo.path())
        .expect("clean repository should remain accepted");
}

#[test]
fn clean_repository_is_verified_before_recording_persistence() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    verify_before_persistence(&snapshot, repo.path())
        .expect("clean repository should be accepted before persistence");

    write_file(repo.path(), "runs.jsonl", "recorded\n");
    assert!(repo.path().join("runs.jsonl").exists());
}

#[test]
fn dirty_repository_that_remains_unchanged_is_accepted_and_records_dirty() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    write_file(repo.path(), "file.moth", "value = 2\n");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");
    assert_eq!(snapshot.git_revision().dirty, Some(true));

    snapshot
        .verify_unchanged(repo.path())
        .expect("dirty but unchanged repository should be accepted");
}

#[test]
fn tracked_file_that_starts_dirty_and_changes_again_is_rejected() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    write_file(repo.path(), "file.moth", "value = 2\n");
    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    write_file(repo.path(), "file.moth", "value = 3\n");

    snapshot
        .verify_unchanged(repo.path())
        .expect_err("second edit to a dirty file should be rejected");
}

#[test]
fn untracked_file_that_starts_present_and_changes_content_is_rejected() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    write_file(repo.path(), "untracked.moth", "value = 1\n");
    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    write_file(repo.path(), "untracked.moth", "value = 2\n");

    snapshot
        .verify_unchanged(repo.path())
        .expect_err("changed untracked file content should be rejected");
}

#[test]
fn tracked_file_modification_is_rejected() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    write_file(repo.path(), "file.moth", "value = 2\n");

    snapshot
        .verify_unchanged(repo.path())
        .expect_err("tracked modification should be rejected");
}

#[test]
fn untracked_file_creation_is_rejected() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    write_file(repo.path(), "new.moth", "value = 1\n");

    snapshot
        .verify_unchanged(repo.path())
        .expect_err("new untracked file should be rejected");
}

#[test]
fn ignored_file_creation_under_target_is_accepted() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    write_file(repo.path(), ".gitignore", "target/\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    write_file(repo.path(), "target/benchmark-work/output.txt", "ignored\n");

    snapshot
        .verify_unchanged(repo.path())
        .expect("ignored target/ file should not count as a change");
}

#[test]
fn commit_change_is_rejected() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    write_file(repo.path(), "file.moth", "value = 2\n");
    commit_all(repo.path(), "second");

    snapshot
        .verify_unchanged(repo.path())
        .expect_err("commit change should be rejected");
}

#[test]
fn operation_failure_plus_repository_mutation_reports_both_causes() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");

    let operation: Result<(), &str> = Err("benchmark failed");
    write_file(repo.path(), "file.moth", "value = 2\n");

    let error = verify_after_operation(&snapshot, repo.path(), operation)
        .expect_err("both failures should be reported");

    assert!(error.contains("benchmark failed"));
    assert!(error.contains("repository also changed"));
}

#[test]
fn start_revision_reaches_git_revision() {
    let repo = init_git_repo();
    write_file(repo.path(), "file.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");
    let revision = snapshot.git_revision();

    assert!(revision.commit.is_some());
    assert_eq!(revision.dirty, Some(false));
}
