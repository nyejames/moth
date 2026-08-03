//! Tests for the shared benchmark suite measurement and presentation path.

use super::*;
use crate::bench_types::BenchmarkGroup;
use crate::benchmark_fingerprint::compute_benchmark_fingerprints;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode,
    BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload, CliBenchmarkCommand,
};
use crate::benchmark_repository::BenchmarkRepositorySnapshot;
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

struct CliFixture {
    temp_dir: TempDir,
}

impl CliFixture {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("temporary directory should exist");
        init_git_repo(temp_dir.path());
        let fixture_path = temp_dir.path().join("benchmarks");
        fs::create_dir_all(&fixture_path).expect("benchmarks directory should be creatable");
        fs::write(fixture_path.join("fixture.moth"), "value = 42\n")
            .expect("fixture should be writable");
        commit_all(temp_dir.path(), "initial");
        Self { temp_dir }
    }

    fn root(&self) -> &Path {
        self.temp_dir.path()
    }

    fn mock_path(&self, name: &str) -> PathBuf {
        let path = self.root().join(name);

        #[cfg(windows)]
        let path = path.with_extension("bat");

        path
    }

    fn workload(&self, entry: &str) -> BenchmarkWorkload {
        BenchmarkWorkload {
            id: entry
                .rsplit('/')
                .next()
                .unwrap_or(entry)
                .trim_end_matches(".moth")
                .to_owned(),
            entry: PathBuf::from(entry),
            entry_kind: if entry.ends_with(".moth") {
                BenchmarkEntryKind::File
            } else {
                BenchmarkEntryKind::Directory
            },
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            fingerprint_roots: vec![PathBuf::from(entry)],
            fingerprint_excludes: Vec::new(),
            generated_output_roots: Vec::new(),
        }
    }

    fn single_cli_manifest(&self) -> BenchmarkManifest {
        BenchmarkManifest {
            workloads: vec![self.workload("benchmarks/fixture.moth")],
            cases: vec![cli_case(
                "fixture_check",
                0,
                CliBenchmarkCommand::Check,
                &[],
            )],
            manifest_path: self.root().join("manifest.toml"),
            repository_root: self.root().to_path_buf(),
        }
    }

    fn prepared(&self) -> PreparedBenchmarkRun {
        let manifest = self.single_cli_manifest();
        let snapshot =
            BenchmarkRepositorySnapshot::capture(self.root()).expect("snapshot should capture");
        let fingerprints =
            compute_benchmark_fingerprints(&manifest).expect("fingerprints should compute");

        PreparedBenchmarkRun {
            manifest,
            snapshot,
            fingerprints,
        }
    }
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

fn cli_case(
    id: &str,
    workload_index: usize,
    command: CliBenchmarkCommand,
    args: &[&str],
) -> BenchmarkCase {
    BenchmarkCase {
        id: id.to_owned(),
        case_index: workload_index,
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

#[cfg(unix)]
fn create_output_executable(path: &Path, stdout: &str, stderr: &str, exit_code: i32) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!("#!/bin/sh\necho '{stdout}'\necho '{stderr}' >&2\nexit {exit_code}\n");
    fs::write(path, script).expect("mock executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("mock metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mock executable should be executable");
}

#[cfg(windows)]
fn create_output_executable(path: &Path, stdout: &str, stderr: &str, exit_code: i32) {
    let script = format!(
        "@echo off\r\n<nul set /p=\"{stdout}\"\r\n<nul set /p=\"{stderr}\" 1>&2\r\nexit /b {exit_code}\r\n"
    );
    fs::write(path, script).expect("mock executable should be written");
}

fn clean_fixed_output() -> &'static str {
    "MOTH_BENCH status errors=0 warnings=0\nMOTH_BENCH timing command.check.total=1ms"
}

#[test]
fn shared_measurement_produces_expected_statistics() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("measure");
    create_output_executable(&compiler, clean_fixed_output(), "", 0);

    let prepared = fixture.prepared();
    let workspace =
        BenchmarkExecutionWorkspace::create(fixture.root()).expect("workspace should be creatable");
    let context = BenchmarkExecutionContext::new(&prepared.manifest, &compiler, &workspace);

    let iterations = NonZeroUsize::new(3).expect("three iterations");
    let results = measure_cases(&context, &prepared, &prepared.manifest.cases, iterations)
        .expect("measurement should succeed");

    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert!(result.mean_ms > 0.0, "wall time should be positive");
    assert!(result.mean_ms.is_finite());
    assert!(result.median_ms.is_finite());
    assert!(result.stddev_ms.is_finite());
    assert_eq!(result.group_name, "core");
    assert_eq!(
        result
            .identity
            .as_ref()
            .expect("identity should exist")
            .workload_id,
        "fixture"
    );
}

#[test]
fn case_result_builder_computes_expected_statistics() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("unused");
    let prepared = fixture.prepared();
    let workspace =
        BenchmarkExecutionWorkspace::create(fixture.root()).expect("workspace should be creatable");
    let context = BenchmarkExecutionContext::new(&prepared.manifest, &compiler, &workspace);

    let durations = vec![10.0, 20.0, 30.0];
    let observations = vec![crate::bench_types::BenchmarkCaseObservations::default()];
    let result = build_case_result(
        &context,
        &prepared,
        &prepared.manifest.cases[0],
        &durations,
        &observations,
    )
    .expect("case result should build");

    assert_eq!(result.mean_ms, 20.0);
    assert_eq!(result.median_ms, 20.0);
    assert!((result.stddev_ms - 8.16496580927726).abs() < 1e-9);
}

#[test]
fn cli_and_frontend_suites_share_one_presentation_owner() {
    // Both suite kinds route through the same shared functions; this test pins
    // the read-only no-write contract for each kind.
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("read-only");
    create_output_executable(&compiler, clean_fixed_output(), "", 0);

    let prepared = fixture.prepared();
    let workspace =
        BenchmarkExecutionWorkspace::create(fixture.root()).expect("workspace should be creatable");
    let context = BenchmarkExecutionContext::new(&prepared.manifest, &compiler, &workspace);

    let iterations = NonZeroUsize::new(1).expect("one iteration");
    let results = measure_cases(&context, &prepared, &prepared.manifest.cases, iterations)
        .expect("measurement should succeed");

    for suite_kind in [
        BenchmarkSuiteKind::EndToEndCli,
        BenchmarkSuiteKind::FrontendPhases,
    ] {
        present_read_only(&results, suite_kind, None, BenchmarkSelection::Full)
            .expect("read-only presentation should succeed without system identity");
        assert!(
            !Path::new("benchmarks/local-data/runs.jsonl").exists(),
            "read-only presentation must not write normal history"
        );
        assert!(
            !Path::new("benchmarks/summaries").exists(),
            "read-only presentation must not write tracked summaries"
        );
    }
}

#[test]
fn previous_run_loader_is_shared_and_returns_none_without_local_history() {
    for suite_kind in [
        BenchmarkSuiteKind::EndToEndCli,
        BenchmarkSuiteKind::FrontendPhases,
    ] {
        let previous = load_previous_cases_for_system("sys-a", suite_kind, None)
            .expect("previous-run lookup should not fail without local history");
        assert!(previous.is_none());
    }
}

#[test]
fn every_accepted_group_parses_with_stable_unique_sort_order() {
    let groups = [
        BenchmarkGroup::Core,
        BenchmarkGroup::Docs,
        BenchmarkGroup::Stress,
        BenchmarkGroup::Module,
        BenchmarkGroup::Parallelism,
        BenchmarkGroup::Borrow,
    ];
    let mut orders = groups
        .iter()
        .map(|group| {
            let spelling = group.persistence_spelling();
            let parsed = BenchmarkGroup::parse_spelling(spelling)
                .expect("every group spelling should parse");
            assert_eq!(parsed, *group);
            assert!(!group.display_label().is_empty());
            group.sort_order()
        })
        .collect::<Vec<_>>();

    orders.sort_unstable();
    orders.dedup();
    assert_eq!(
        orders.len(),
        groups.len(),
        "group sort orders must be unique"
    );
    assert_eq!(
        groups
            .iter()
            .map(|group| group.persistence_spelling())
            .collect::<Vec<_>>(),
        ["core", "docs", "stress", "module", "parallelism", "borrow"]
    );
}

#[test]
fn group_persistence_spelling_round_trips() {
    for group in [
        BenchmarkGroup::Core,
        BenchmarkGroup::Docs,
        BenchmarkGroup::Stress,
        BenchmarkGroup::Module,
        BenchmarkGroup::Parallelism,
        BenchmarkGroup::Borrow,
    ] {
        let spelling = group.persistence_spelling();
        assert_eq!(
            BenchmarkGroup::parse_spelling(spelling),
            Some(group),
            "persistence spelling should round-trip for {group:?}"
        );
    }
    assert_eq!(BenchmarkGroup::parse_spelling("unknown"), None);
}
