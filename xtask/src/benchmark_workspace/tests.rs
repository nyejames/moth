use super::*;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkManifest,
    BenchmarkManifestError, BenchmarkRunner, BenchmarkWorkload, CliBenchmarkCommand,
};
use std::path::PathBuf;
use tempfile::tempdir;

fn make_workload(id: &str, entry: &str, entry_kind: BenchmarkEntryKind) -> BenchmarkWorkload {
    BenchmarkWorkload {
        id: id.to_owned(),
        entry: PathBuf::from(entry),
        entry_kind,
        fingerprint_roots: vec![PathBuf::from(entry)],
        fingerprint_excludes: Vec::new(),
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
        workload_index,
        group_name: "core".to_owned(),
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Cli {
            command,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        },
    }
}

fn make_manifest(
    repository_root: &std::path::Path,
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
        vec![make_workload(
            "first",
            "first.moth",
            BenchmarkEntryKind::File,
        )],
        vec![
            cli_case("first_check", 0, CliBenchmarkCommand::Check, &[]),
            cli_case("first_build", 0, CliBenchmarkCommand::Build, &[]),
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
fn directory_entry_case_keeps_repository_root_as_current_directory() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::create_dir_all(directory.path().join("project"))
        .expect("project directory should be creatable");
    std::fs::write(directory.path().join("project/main.moth"), "value = 42\n")
        .expect("project fixture should be writable");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "project",
            "project",
            BenchmarkEntryKind::Directory,
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

    let invocation = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("invocation should resolve");

    assert_eq!(invocation.current_directory, directory.path());
    assert_eq!(invocation.args[0], "project");
}

#[test]
fn authored_runner_arguments_remain_ordered_after_entry_argument() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("fixture.moth"), "value = 42\n")
        .expect("fixture should be writable");

    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "fixture",
            "fixture.moth",
            BenchmarkEntryKind::File,
        )],
        vec![cli_case(
            "fixture_build",
            0,
            CliBenchmarkCommand::Build,
            &["--release", "--terse"],
        )],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let invocation = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("invocation should resolve");

    assert_eq!(invocation.command, CliBenchmarkCommand::Build);
    assert!(invocation.args[0].ends_with("fixture.moth"));
    assert_eq!(invocation.args[1], "--release");
    assert_eq!(invocation.args[2], "--terse");
}

#[test]
fn frontend_cases_cannot_request_a_cli_invocation() {
    let directory = tempdir().expect("temporary repository should exist");
    std::fs::write(directory.path().join("fixture.moth"), "value = 42\n")
        .expect("fixture should be writable");

    let frontend_case = BenchmarkCase {
        id: "frontend_case".to_owned(),
        workload_index: 0,
        group_name: "core".to_owned(),
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Frontend {
            profile: crate::benchmark_manifest::FrontendBenchmarkProfile::Dev,
        },
    };
    let manifest = make_manifest(
        directory.path(),
        vec![make_workload(
            "fixture",
            "fixture.moth",
            BenchmarkEntryKind::File,
        )],
        vec![frontend_case],
    );
    let workspace = BenchmarkExecutionWorkspace::create(directory.path())
        .expect("workspace should be creatable");

    let error = workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect_err("frontend case must not resolve a CLI invocation");

    assert!(matches!(error, BenchmarkManifestError::Invalid { .. }));
    assert!(
        error
            .to_string()
            .contains("case does not declare a CLI runner")
    );
}
