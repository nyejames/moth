//! Tests for the run-scoped benchmark workspace and explicit output lifecycle.

use super::*;
use crate::bench_types::BenchmarkGroup;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode,
    BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload, CliBenchmarkCommand,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn make_workload(
    id: &str,
    entry: &str,
    entry_kind: BenchmarkEntryKind,
    generated_output_roots: &[&str],
) -> BenchmarkWorkload {
    BenchmarkWorkload {
        id: id.to_owned(),
        entry: PathBuf::from(entry),
        entry_kind,
        fingerprint_mode: BenchmarkFingerprintMode::FullTree,
        fingerprint_roots: vec![PathBuf::from(entry)],
        fingerprint_excludes: Vec::new(),
        generated_output_roots: generated_output_roots.iter().map(PathBuf::from).collect(),
    }
}

fn cli_case(
    id: &str,
    workload_index: usize,
    command: CliBenchmarkCommand,
    args: &[&str],
) -> BenchmarkCase {
    BenchmarkCase {
        id: id.to_owned(),
        case_index: 0,
        workload_index,
        group_name: BenchmarkGroup::Core,
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Cli {
            command,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        },
    }
}

fn make_manifest(
    repository_root: &Path,
    workloads: Vec<BenchmarkWorkload>,
    cases: Vec<BenchmarkCase>,
) -> BenchmarkManifest {
    BenchmarkManifest {
        workloads,
        cases,
        manifest_path: repository_root.join("manifest.toml"),
        repository_root: repository_root.to_path_buf(),
    }
}

fn directory_project(root: &Path, name: &str) -> PathBuf {
    let entry_path = root.join(name);
    std::fs::create_dir_all(&entry_path).expect("project directory should be creatable");
    std::fs::write(entry_path.join("main.moth"), "value = 42\n")
        .expect("project fixture should be writable");
    entry_path
}

#[test]
fn file_entry_case_resolves_to_absolute_entry_argument() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("fixture.moth"), "value = 42\n")
        .expect("fixture should be writable");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "fixture",
            "fixture.moth",
            BenchmarkEntryKind::File,
            &[],
        )],
        vec![cli_case(
            "fixture_check",
            0,
            CliBenchmarkCommand::Check,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let invocation = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("invocation should resolve");

    let expected_entry = directory.path().join("fixture.moth");
    assert_eq!(invocation.args[0], expected_entry.display().to_string());
}

#[test]
fn file_entry_case_uses_directory_below_benchmark_work() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("fixture.moth"), "value = 42\n")
        .expect("fixture should be writable");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "fixture",
            "fixture.moth",
            BenchmarkEntryKind::File,
            &[],
        )],
        vec![cli_case(
            "fixture_check",
            0,
            CliBenchmarkCommand::Check,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let invocation = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("invocation should resolve");

    assert!(
        invocation
            .current_directory
            .starts_with(directory.path().join("target").join("benchmark-work"))
    );
    assert!(invocation.current_directory.ends_with("fixture_check"));
}

#[test]
fn repeated_resolution_for_one_case_returns_the_same_directory() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("fixture.moth"), "value = 42\n")
        .expect("fixture should be writable");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "fixture",
            "fixture.moth",
            BenchmarkEntryKind::File,
            &[],
        )],
        vec![cli_case(
            "fixture_check",
            0,
            CliBenchmarkCommand::Check,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let first = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("first invocation should resolve");
    let second = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("second invocation should resolve");

    assert_eq!(first.current_directory, second.current_directory);
}

#[test]
fn two_cases_use_distinct_case_directories() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("first.moth"), "value = 1\n")
        .expect("first fixture should be writable");
    std::fs::write(directory.path().join("second.moth"), "value = 2\n")
        .expect("second fixture should be writable");

    let manifest = make_manifest(
        directory.path(),
        vec![
            make_workload("first", "first.moth", BenchmarkEntryKind::File, &[]),
            make_workload("second", "second.moth", BenchmarkEntryKind::File, &[]),
        ],
        vec![
            cli_case("first_check", 0, CliBenchmarkCommand::Check, &[]),
            cli_case("second_check", 1, CliBenchmarkCommand::Check, &[]),
        ],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let first = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("first invocation should resolve");
    let second = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[1])
        .expect("second invocation should resolve");

    assert_ne!(first.current_directory, second.current_directory);
}

#[test]
fn directory_build_case_registers_declared_roots_and_finish_removes_them() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev", "release"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");

    // Simulate the compiler writing its declared outputs.
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");
    std::fs::create_dir_all(entry_path.join("release"))
        .expect("release output should be creatable");

    workspace
        .finish()
        .expect("finish should remove registered run-owned roots");

    assert!(!entry_path.join("dev").exists());
    assert!(!entry_path.join("release").exists());
}

#[test]
fn check_case_registers_no_root() {
    let directory = tempdir().expect("temporary repository should exist");
    directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_check",
            0,
            CliBenchmarkCommand::Check,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("check invocation should resolve");

    workspace
        .finish()
        .expect("finish with no registered roots should succeed");
    assert!(workspace.registered_roots.borrow().is_empty());
}

#[test]
fn frontend_case_registers_no_root() {
    let directory = tempdir().expect("temporary repository should exist");
    directory_project(directory.path(), "project");

    let frontend_case = BenchmarkCase {
        id: "project_frontend".to_owned(),
        case_index: 0,
        workload_index: 0,
        group_name: BenchmarkGroup::Core,
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Frontend {
            profile: crate::benchmark_manifest::FrontendBenchmarkProfile::Dev,
        },
    };
    let _manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![frontend_case],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    // Frontend cases never resolve a CLI invocation, so no root is registered.
    workspace
        .finish()
        .expect("finish with no registered roots should succeed");
    assert!(workspace.registered_roots.borrow().is_empty());
}

#[test]
fn existing_output_root_is_rejected_before_execution() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");
    std::fs::create_dir_all(entry_path.join("dev")).expect("pre-existing dev should exist");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let error = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect_err("a pre-existing output root must be rejected");

    assert!(error.to_string().contains("must not exist before the run"));
    assert!(workspace.registered_roots.borrow().is_empty());
}

fn init_git_repo(root: &Path) {
    for args in [
        &["init"][..],
        &["config", "user.email", "test@example.com"][..],
        &["config", "user.name", "Test"][..],
    ] {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git command should succeed");
        assert!(output.status.success(), "git {args:?} failed");
    }
}

fn commit_all(root: &Path, message: &str) {
    for args in [&["add", "-A"][..], &["commit", "-m", message][..]] {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git command should succeed");
        assert!(output.status.success(), "git {args:?} failed");
    }
}

#[test]
fn tracked_output_root_is_rejected_and_never_deleted() {
    let directory = tempdir().expect("temporary repository should exist");
    init_git_repo(directory.path());
    let entry_path = directory_project(directory.path(), "project");
    std::fs::create_dir_all(entry_path.join("dev")).expect("tracked dev should exist");
    std::fs::write(entry_path.join("dev/kept.txt"), "user data\n")
        .expect("tracked file should be writable");
    commit_all(directory.path(), "track dev output");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let error = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect_err("a tracked output root must be rejected");

    assert!(error.to_string().contains("tracked by Git"));
    assert!(entry_path.join("dev/kept.txt").exists());
}

#[test]
fn finish_is_idempotent() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");

    workspace.finish().expect("first finish should succeed");
    workspace
        .finish()
        .expect("second finish must be idempotent");
    assert!(!entry_path.join("dev").exists());
}

#[test]
fn drop_is_not_required_for_a_successful_run() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");

    workspace
        .finish()
        .expect("explicit finish must define success");
    assert!(!entry_path.join("dev").exists());

    // Dropping afterwards must not change anything.
    drop(workspace);
    assert!(!entry_path.join("dev").exists());
}

#[test]
#[cfg(unix)]
fn symlink_replaced_root_fails_finalisation() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");

    // Replace the root with a symlink pointing elsewhere.
    let outside = directory.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside directory should be creatable");
    std::fs::remove_dir_all(entry_path.join("dev")).expect("remove dev");
    std::os::unix::fs::symlink(&outside, entry_path.join("dev"))
        .expect("symlink should be creatable");

    let error = workspace
        .finish()
        .expect_err("a symlink-replaced root must fail finalisation");
    assert!(matches!(
        error,
        BenchmarkWorkspaceError::SymlinkReplacedRoot { .. }
    ));
}

#[test]
#[cfg(unix)]
fn removal_failure_blocks_persistence_path() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    let dev_path = entry_path.join("dev");
    std::fs::create_dir_all(&dev_path).expect("dev output should be creatable");
    std::fs::write(dev_path.join("artifact.txt"), "x").expect("artifact should be writable");
    let mut permissions = std::fs::metadata(&dev_path)
        .expect("metadata should be readable")
        .permissions();
    permissions.set_mode(0o000);
    std::fs::set_permissions(&dev_path, permissions).expect("permissions should be settable");

    let error = workspace
        .finish()
        .expect_err("a removal failure must block finalisation");
    assert!(matches!(
        error,
        BenchmarkWorkspaceError::RemovalFailed { .. }
    ));

    // Restore permissions so the temporary directory can be cleaned up.
    let mut permissions = std::fs::metadata(&dev_path)
        .expect("metadata should be readable")
        .permissions();
    permissions.set_mode(0o700);
    let _ = std::fs::set_permissions(&dev_path, permissions);
}

#[test]
fn undeclared_manifest_fails_finalisation() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");
    std::fs::create_dir_all(entry_path.join("nested"))
        .expect("nested directory should be creatable");
    std::fs::write(entry_path.join("nested/.moth_manifest"), "drift\n")
        .expect("undeclared manifest should be writable");

    let error = workspace
        .finish()
        .expect_err("an undeclared output manifest must fail finalisation");
    assert!(matches!(
        error,
        BenchmarkWorkspaceError::UndeclaredManifest { .. }
    ));
}

#[test]
fn per_execution_check_detects_entry_level_manifest() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::write(entry_path.join(".moth_manifest"), "drift\n")
        .expect("undeclared manifest should be writable");

    let error = workspace
        .check_directory_build_output(&entry_path)
        .expect_err("an entry-level manifest must fail the per-execution check");
    assert!(matches!(
        error,
        BenchmarkWorkspaceError::UndeclaredManifest { .. }
    ));
}

#[test]
fn finalise_reports_operation_and_cleanup_failures_together() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");
    std::fs::write(entry_path.join(".moth_manifest"), "drift\n")
        .expect("undeclared manifest should be writable");

    let error = finalise_workspace::<()>(&workspace, Err("operation failed".to_owned()))
        .expect_err("both failures must be reported");
    assert!(error.contains("operation failed"));
    assert!(error.contains("workspace cleanup also failed"));
}

#[test]
fn finalise_workspace_preserves_successful_operation_value() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");
    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");
    std::fs::create_dir_all(entry_path.join("dev")).expect("dev output should be creatable");

    let value = finalise_workspace(&workspace, Ok(42usize))
        .expect("a successful operation with successful cleanup should pass through");
    assert_eq!(value, 42);
    assert!(!entry_path.join("dev").exists());
}

#[test]
#[cfg(unix)]
fn scan_rejects_symlink_encountered_during_undeclared_manifest_scan() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["dev"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");
    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");

    let outside = directory.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside directory should be creatable");
    std::os::unix::fs::symlink(&outside, entry_path.join("linked"))
        .expect("nested symlink should be creatable");

    let error = workspace
        .finish()
        .expect_err("a symlink inside the scan must fail finalisation");
    assert!(matches!(
        error,
        BenchmarkWorkspaceError::UnexpectedSymlink { .. }
    ));
}

#[test]
#[cfg(unix)]
fn nested_root_with_symlink_intermediate_component_fails_finalisation() {
    let directory = tempdir().expect("temporary repository should exist");
    let entry_path = directory_project(directory.path(), "project");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
            &["deep/assets", "deep"],
        )],
        vec![cli_case(
            "project_build",
            0,
            CliBenchmarkCommand::Build,
            &[],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");
    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should resolve");

    let outside = directory.path().join("outside");
    std::fs::create_dir_all(outside.join("assets")).expect("outside assets should be creatable");
    std::os::unix::fs::symlink(&outside, entry_path.join("deep"))
        .expect("intermediate symlink should be creatable");

    let error = workspace
        .finish()
        .expect_err("a symlink component between entry and root must fail finalisation");
    assert!(matches!(
        error,
        BenchmarkWorkspaceError::SymlinkComponent { .. }
    ));
}
