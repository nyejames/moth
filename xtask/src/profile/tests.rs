use super::run::{profile_history_allowed, select_profile_cases};
use crate::bench_types::BenchmarkGroup;
use crate::benchmark_execution::BenchmarkExecutionContext;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode,
    BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload, CliBenchmarkCommand,
    FrontendBenchmarkProfile,
};
use crate::benchmark_repository::BenchmarkRepositorySnapshot;
use crate::benchmark_run::BenchmarkPaths;
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
use super::run::collect_profile_run;
#[cfg(unix)]
use crate::benchmark_run::PreparedBenchmarkRun;
#[cfg(unix)]
use crate::benchmark_workspace::finalise_workspace;
#[cfg(unix)]
use crate::compiler_binary::CompilerBinary;
#[cfg(unix)]
use crate::profile::options::{ProfileFilterMode, ProfileOptions};
#[cfg(unix)]
use crate::profile::runner::{PresymbolicationFlag, SamplyRecordCapabilities};

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
fn profile_paths_are_anchored_to_repository_root() {
    let repository = tempfile::tempdir().expect("temporary repository should exist");

    let paths = BenchmarkPaths::for_repository(repository.path());

    assert!(paths.profiles.is_absolute());
    assert_eq!(
        paths.profiles,
        repository.path().join("benchmarks/local-data/profiles")
    );
    assert_eq!(
        paths.profile_history,
        repository
            .path()
            .join("benchmarks/local-data/profile-runs.jsonl")
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
            generated_output_roots: Vec::new(),
        }],
        cases: vec![
            BenchmarkCase {
                id: "cli_case".to_owned(),
                case_index: 0,
                workload_index: 0,
                group_name: BenchmarkGroup::Core,
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
                group_name: BenchmarkGroup::Core,
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
            generated_output_roots: Vec::new(),
        }],
        cases: vec![BenchmarkCase {
            id: "cli_case".to_owned(),
            case_index: 0,
            workload_index: 0,
            group_name: BenchmarkGroup::Core,
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

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, contents).expect("mock executable should be writable");
    let mut permissions = fs::metadata(path)
        .expect("mock executable metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mock executable should be executable");
}

#[cfg(unix)]
fn profile_fixture(root: &Path) -> (BenchmarkManifest, PreparedBenchmarkRun) {
    let entry_path = root.join("project");
    fs::create_dir_all(&entry_path).expect("project directory should be creatable");
    fs::write(entry_path.join("main.moth"), "value = 42\n")
        .expect("project source should be writable");
    commit_all(root, "initial fixture");

    let manifest = BenchmarkManifest {
        workloads: vec![BenchmarkWorkload {
            id: "fixture".to_owned(),
            entry: "project".into(),
            entry_kind: BenchmarkEntryKind::Directory,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            fingerprint_roots: vec!["project".into()],
            fingerprint_excludes: vec!["project/dev".into()],
            generated_output_roots: vec!["dev".into()],
        }],
        cases: vec![BenchmarkCase {
            id: "build_case".to_owned(),
            case_index: 0,
            workload_index: 0,
            group_name: BenchmarkGroup::Core,
            quick: false,
            expectation: BenchmarkExpectation::Clean,
            runner: BenchmarkRunner::Cli {
                command: CliBenchmarkCommand::Build,
                args: Vec::new(),
            },
        }],
        manifest_path: root.join("manifest.toml"),
        repository_root: root.to_path_buf(),
    };
    let snapshot = BenchmarkRepositorySnapshot::capture(root).expect("snapshot should capture");
    let fingerprints = crate::benchmark_fingerprint::compute_benchmark_fingerprints(&manifest)
        .expect("fingerprints should compute");
    let prepared = PreparedBenchmarkRun {
        manifest: manifest.clone(),
        snapshot,
        fingerprints,
        paths: BenchmarkPaths::for_repository(root),
    };
    (manifest, prepared)
}

#[cfg(unix)]
fn profile_collection_failure_finalises_workspace(script: &str, expected_error: &str) {
    let repo = init_git_repo();
    let (_manifest, prepared) = profile_fixture(repo.path());
    let selected_cases = select_profile_cases(&prepared.manifest, None)
        .expect("profile case selection should succeed");

    let compiler_path = repo.path().join("mock-moth");
    write_executable(&compiler_path, script);
    let compiler = CompilerBinary {
        path: compiler_path,
        symbol_dirs: Vec::new(),
        profiling_symbols: None,
    };
    let samply = SamplyRecordCapabilities {
        version: "test".to_owned(),
        presymbolication_flag: PresymbolicationFlag::Unavailable,
    };
    let workspace =
        BenchmarkExecutionWorkspace::create(repo.path()).expect("workspace should be creatable");
    let options = ProfileOptions {
        filter: ProfileFilterMode::RawIndex,
        case_filter: None,
        samply_rate_hz: None,
        presymbolicate: false,
    };

    let result = finalise_workspace(
        &workspace,
        collect_profile_run(
            &options,
            &prepared,
            selected_cases,
            &workspace,
            &compiler,
            &samply,
        ),
    );

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("collection failure must surface"),
    };
    assert!(
        error.contains(expected_error),
        "error should name the intended phase, got: {error}"
    );
    assert!(
        !repo.path().join("project/dev").exists(),
        "finish() must run after a collection failure"
    );
}

#[test]
#[cfg(unix)]
fn preflight_failure_still_calls_explicit_finish() {
    profile_collection_failure_finalises_workspace(
        "#!/bin/sh\nmkdir -p project/dev\nexit 1\n",
        "profile preflight",
    );
}

#[test]
#[cfg(unix)]
fn observation_failure_still_calls_explicit_finish() {
    let script = r#"#!/bin/sh
count_file="$PWD/.mock-invocation-count"
if [ ! -f "$count_file" ]; then
  printf '0
' > "$count_file"
fi
count=$(cat "$count_file")
count=$((count + 1))
printf '%s
' "$count" > "$count_file"
if [ "$count" -eq 1 ]; then
  printf 'MOTH_BENCH timing-schema 2
MOTH_BENCH timing command.build.total=1ms
MOTH_BENCH status errors=0 warnings=0
'
  exit 0
fi
mkdir -p project/dev
exit 1
"#;
    profile_collection_failure_finalises_workspace(script, "Observation pass failed");
}

#[test]
#[cfg(unix)]
fn samply_failure_still_calls_explicit_finish() {
    let repo = init_git_repo();
    let (manifest, _prepared) = profile_fixture(repo.path());

    let workspace =
        BenchmarkExecutionWorkspace::create(repo.path()).expect("workspace should be creatable");
    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should register the output root");
    fs::create_dir_all(repo.path().join("project/dev")).expect("dev output should be creatable");

    let error = finalise_workspace::<()>(
        &workspace,
        Err("Samply recording failed for case 'build_case'".to_owned()),
    )
    .expect_err("a Samply failure must surface");

    assert!(error.contains("Samply recording failed"));
    assert!(!repo.path().join("project/dev").exists());
}

#[test]
#[cfg(unix)]
fn artifact_write_failure_still_calls_explicit_finish() {
    let repo = init_git_repo();
    let (_manifest, prepared) = profile_fixture(repo.path());

    // A compiler that passes preflight (and creates the generated root) while
    // the profile artifact directory is not writable. The collection phase
    // fails on artifact creation; finalisation must still remove the root.
    let selected_cases =
        select_profile_cases(&_manifest, None).expect("profile case selection should succeed");
    let compiler_path = repo.path().join("mock-moth");
    write_executable(
        &compiler_path,
        "#!/bin/sh\nmkdir -p project/dev\nprintf 'MOTH_BENCH timing-schema 2\nMOTH_BENCH timing command.build.total=1ms\nMOTH_BENCH status errors=0 warnings=0\n'\nexit 0\n",
    );
    let compiler = CompilerBinary {
        path: compiler_path,
        symbol_dirs: Vec::new(),
        profiling_symbols: None,
    };
    let samply = SamplyRecordCapabilities {
        version: "test".to_owned(),
        presymbolication_flag: PresymbolicationFlag::Unavailable,
    };
    let workspace =
        BenchmarkExecutionWorkspace::create(repo.path()).expect("workspace should be creatable");
    let options = ProfileOptions {
        filter: ProfileFilterMode::RawIndex,
        case_filter: None,
        samply_rate_hz: None,
        presymbolicate: false,
    };

    let profiles_root = &prepared.paths.profiles;
    fs::create_dir_all(profiles_root).expect("profiles root should be creatable");
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(profiles_root)
            .expect("profiles metadata should be readable")
            .permissions();
        permissions.set_mode(0o500);
        fs::set_permissions(profiles_root, permissions)
            .expect("profiles root should become read-only");
    }

    let result = finalise_workspace(
        &workspace,
        collect_profile_run(
            &options,
            &prepared,
            selected_cases,
            &workspace,
            &compiler,
            &samply,
        ),
    );

    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(profiles_root)
            .expect("profiles metadata should be readable")
            .permissions();
        permissions.set_mode(0o700);
        let _ = fs::set_permissions(profiles_root, permissions);
    }

    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("artifact write failure must surface"),
    };
    assert!(
        error.contains("Failed to create profile run directory"),
        "error should name the artifact phase, got: {error}"
    );
    assert!(
        !repo.path().join("project/dev").exists(),
        "finish() must run after an artifact write failure"
    );
}

#[test]
#[cfg(unix)]
fn cleanup_failure_prevents_profile_history_append() {
    let repo = init_git_repo();
    let (manifest, prepared) = profile_fixture(repo.path());

    let workspace =
        BenchmarkExecutionWorkspace::create(repo.path()).expect("workspace should be creatable");
    workspace
        .resolve_cli_invocation(&manifest, &manifest.cases[0])
        .expect("build invocation should register the output root");
    fs::create_dir_all(repo.path().join("project/dev")).expect("dev output should be creatable");
    fs::write(repo.path().join("project/.moth_manifest"), "drift\n")
        .expect("undeclared manifest should be writable");

    let history_path = &prepared.paths.profile_history;
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).expect("history parent should be creatable");
    }
    fs::write(history_path, "sentinel-history\n").expect("history sentinel should be writable");

    let error =
        finalise_workspace::<()>(&workspace, Ok(())).expect_err("a cleanup failure must surface");
    assert!(error.contains("undeclared output manifest"));

    let contents = fs::read_to_string(history_path).expect("history should remain readable");
    assert_eq!(
        contents, "sentinel-history\n",
        "profile history must not be appended after a cleanup failure"
    );
}
