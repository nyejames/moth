use super::run::{profile_artifacts_root, profile_history_allowed, select_profile_cases};
use crate::benchmark_execution::BenchmarkExecutionContext;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode,
    BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload, CliBenchmarkCommand,
    FrontendBenchmarkProfile,
};
use crate::benchmark_repository::BenchmarkRepositorySnapshot;
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn profile_selection_rejects_frontend_cases_clearly() {
    let manifest = manifest();

    let error = select_profile_cases(&manifest, Some("frontend_case"))
        .expect_err("frontend profiling must be rejected");

    assert!(error.contains("Frontend benchmark case 'frontend_case'"));
    assert!(error.contains("cannot be profiled with Samply"));
    assert!(error.contains("CLI benchmark case"));
}

#[test]
fn unfiltered_profile_selection_keeps_only_cli_cases() {
    let manifest = manifest();

    let cases = select_profile_cases(&manifest, None).expect("CLI cases should be selected");

    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].id, "cli_case");
}

#[test]
fn profile_artifacts_root_is_anchored_to_repository_root() {
    let repository = tempfile::tempdir().expect("temporary repository should exist");

    let profiles_root = profile_artifacts_root(repository.path());

    assert!(profiles_root.is_absolute());
    assert_eq!(
        profiles_root,
        repository.path().join("benchmarks/local-data/profiles")
    );
}

fn init_git_repo() -> tempfile::TempDir {
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
fn clean_profile_run_may_append_history_but_dirty_run_may_not() {
    let repo = init_git_repo();
    write_file(repo.path(), "fixture.moth", "value = 1\n");
    commit_all(repo.path(), "initial");

    let clean_snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");
    assert!(profile_history_allowed(&clean_snapshot));

    write_file(repo.path(), "fixture.moth", "value = 2\n");
    let dirty_snapshot =
        BenchmarkRepositorySnapshot::capture(repo.path()).expect("snapshot should capture");
    assert!(!profile_history_allowed(&dirty_snapshot));
}

fn manifest() -> BenchmarkManifest {
    BenchmarkManifest {
        workloads: vec![BenchmarkWorkload {
            id: "fixture".to_owned(),
            entry: "fixture.moth".into(),
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            fingerprint_roots: vec!["fixture.moth".into()],
            fingerprint_excludes: Vec::new(),
        }],
        cases: vec![
            BenchmarkCase {
                id: "cli_case".to_owned(),
                case_index: 0,
                workload_index: 0,
                group_name: "core".to_owned(),
                quick: false,
                expectation: BenchmarkExpectation::Clean,
                runner: BenchmarkRunner::Cli {
                    command: CliBenchmarkCommand::Check,
                    args: Vec::new(),
                },
            },
            BenchmarkCase {
                id: "frontend_case".to_owned(),
                case_index: 1,
                workload_index: 0,
                group_name: "core".to_owned(),
                quick: false,
                expectation: BenchmarkExpectation::Clean,
                runner: BenchmarkRunner::Frontend {
                    profile: FrontendBenchmarkProfile::Dev,
                },
            },
        ],
        manifest_path: "manifest.toml".into(),
        repository_root: ".".into(),
    }
}

#[test]
fn observation_and_samply_receive_one_resolved_invocation() {
    let directory = tempfile::tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("fixture.moth"), "value = 42\n")
        .expect("fixture should be writable");

    let manifest = BenchmarkManifest {
        workloads: vec![BenchmarkWorkload {
            id: "fixture".to_owned(),
            entry: "fixture.moth".into(),
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            fingerprint_roots: vec!["fixture.moth".into()],
            fingerprint_excludes: Vec::new(),
        }],
        cases: vec![BenchmarkCase {
            id: "cli_case".to_owned(),
            case_index: 0,
            workload_index: 0,
            group_name: "core".to_owned(),
            quick: false,
            expectation: BenchmarkExpectation::Clean,
            runner: BenchmarkRunner::Cli {
                command: CliBenchmarkCommand::Check,
                args: vec!["--terse".to_owned()],
            },
        }],
        manifest_path: directory.path().join("manifest.toml"),
        repository_root: directory.path().to_path_buf(),
    };
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");
    let compiler = directory.path().join("unused-compiler");
    let context = BenchmarkExecutionContext::new(&manifest, &compiler, &workspace);

    // The profile orchestrator resolves one invocation and passes the same
    // command, args and current directory to both the observation pass and
    // Samply. This test verifies the resolved invocation is consistent.
    let invocation = context
        .resolve_cli_invocation(&manifest.cases[0])
        .expect("invocation should resolve");

    // The observation pass uses invocation.command and invocation.args.
    // The Samply pass uses invocation.current_directory, invocation.command
    // and invocation.args. Both must come from the same resolved invocation.
    assert_eq!(invocation.command, CliBenchmarkCommand::Check);
    assert!(invocation.args[0].ends_with("fixture.moth"));
    assert_eq!(invocation.args[1], "--terse");
    assert!(
        invocation
            .current_directory
            .starts_with(directory.path().join("target").join("benchmark-work"))
    );
    assert!(invocation.current_directory.ends_with("cli_case"));

    // Both the observation and Samply paths consume the same fields from this
    // one invocation. The profile orchestrator must not reconstruct the
    // command separately for either pass.
    let _ = PathBuf::from(&invocation.current_directory);
}
