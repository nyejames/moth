//! Focused contracts for deterministic workload identity and filesystem policy.

use super::*;
use crate::bench_types::{
    BenchmarkCaseObservations, BenchmarkCaseResult, BenchmarkComparison, BenchmarkGroup,
    BenchmarkMeasurementIdentity,
};
use crate::benchmark_manifest::{
    BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode, CliBenchmarkCommand,
    FrontendBenchmarkProfile,
};
use std::fs;
use tempfile::{TempDir, tempdir};

fn write_file(repository_root: &Path, relative_path: &str, contents: &[u8]) {
    let path = repository_root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("test file parent should be creatable");
    }
    fs::write(path, contents).expect("test file should be writable");
}

fn cli_runner(command: CliBenchmarkCommand, args: &[&str]) -> BenchmarkRunner {
    BenchmarkRunner::Cli {
        command,
        args: args.iter().map(|argument| (*argument).to_owned()).collect(),
    }
}

fn frontend_runner() -> BenchmarkRunner {
    BenchmarkRunner::Frontend {
        profile: FrontendBenchmarkProfile::Dev,
    }
}

fn manifest(
    repository_root: &Path,
    entry: &str,
    roots: &[&str],
    excludes: &[&str],
    runners: Vec<BenchmarkRunner>,
) -> BenchmarkManifest {
    let cases = runners
        .into_iter()
        .enumerate()
        .map(|(index, runner)| BenchmarkCase {
            id: format!("case_{index}"),
            case_index: index,
            workload_index: 0,
            group_name: BenchmarkGroup::Core,
            quick: false,
            expectation: BenchmarkExpectation::Clean,
            runner,
        })
        .collect();

    BenchmarkManifest {
        workloads: vec![BenchmarkWorkload {
            id: "workload".to_owned(),
            entry: entry.into(),
            entry_kind: if entry.ends_with(".moth") {
                BenchmarkEntryKind::File
            } else {
                BenchmarkEntryKind::Directory
            },
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            fingerprint_roots: roots.iter().map(PathBuf::from).collect(),
            fingerprint_excludes: excludes.iter().map(PathBuf::from).collect(),
            generated_output_roots: Vec::new(),
        }],
        cases,
        manifest_path: repository_root.join("benchmarks/manifest.toml"),
        repository_root: repository_root.to_owned(),
    }
}

fn standard_manifest(repository_root: &Path) -> BenchmarkManifest {
    manifest(
        repository_root,
        "project/main.moth",
        &["project"],
        &["project/dev"],
        vec![cli_runner(CliBenchmarkCommand::Check, &[])],
    )
}

fn source_fingerprint(manifest: &BenchmarkManifest) -> SourceWorkloadFingerprint {
    let fingerprints =
        compute_benchmark_fingerprints(manifest).expect("benchmark fingerprints should compute");
    assert_eq!(fingerprints.workloads.len(), manifest.workloads.len());
    fingerprints.workloads[0]
}

fn measurement_fingerprint(manifest: &BenchmarkManifest) -> CaseMeasurementFingerprint {
    let fingerprints =
        compute_benchmark_fingerprints(manifest).expect("benchmark fingerprints should compute");
    assert_eq!(fingerprints.cases.len(), manifest.cases.len());
    fingerprints.cases[0]
}

fn repository_with_files(files: &[(&str, &[u8])]) -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    for (path, contents) in files {
        write_file(repository.path(), path, contents);
    }
    repository
}

fn case_with_indices(workload_index: usize, case_index: usize) -> BenchmarkCase {
    BenchmarkCase {
        id: "fixture_case".to_string(),
        case_index,
        workload_index,
        group_name: BenchmarkGroup::Core,
        quick: true,
        expectation: BenchmarkExpectation::Clean,
        runner: cli_runner(CliBenchmarkCommand::Check, &[]),
    }
}

fn identity_fingerprints() -> BenchmarkFingerprints {
    BenchmarkFingerprints {
        workloads: vec![SourceWorkloadFingerprint {
            first_lane: 1,
            second_lane: 2,
        }],
        cases: vec![CaseMeasurementFingerprint {
            first_lane: 3,
            second_lane: 4,
        }],
    }
}

#[test]
fn checked_identity_construction_completes_for_a_valid_case() {
    let repository = repository_with_files(&[("project/main.moth", b"value = 1\n")]);
    let manifest = standard_manifest(repository.path());
    let case = case_with_indices(0, 0);
    let fingerprints = identity_fingerprints();

    let identity = fingerprints
        .identity_for(&manifest, &case)
        .expect("complete identity should construct");

    assert_eq!(identity.workload_id, "workload");
    assert_eq!(
        identity.source_fingerprint,
        "00000000000000010000000000000002"
    );
    assert_eq!(
        identity.measurement_fingerprint,
        "00000000000000030000000000000004"
    );
}

#[test]
fn checked_identity_rejects_missing_workload_relationship() {
    let repository = repository_with_files(&[("project/main.moth", b"value = 1\n")]);
    let manifest = standard_manifest(repository.path());
    let case = case_with_indices(5, 0);
    let fingerprints = identity_fingerprints();

    let error = fingerprints
        .identity_for(&manifest, &case)
        .expect_err("an invalid workload relationship must fail");

    assert!(matches!(
        error,
        BenchmarkIdentityError::InvalidWorkloadRelationship { .. }
    ));
}

#[test]
fn checked_identity_rejects_missing_source_fingerprint() {
    let repository = repository_with_files(&[("project/main.moth", b"value = 1\n")]);
    let manifest = standard_manifest(repository.path());
    let case = case_with_indices(0, 0);
    let fingerprints = BenchmarkFingerprints {
        workloads: Vec::new(),
        cases: identity_fingerprints().cases,
    };

    let error = fingerprints
        .identity_for(&manifest, &case)
        .expect_err("a missing source fingerprint must fail");

    assert!(matches!(
        error,
        BenchmarkIdentityError::MissingWorkloadFingerprint { .. }
    ));
}

#[test]
fn checked_identity_rejects_missing_measurement_fingerprint() {
    let repository = repository_with_files(&[("project/main.moth", b"value = 1\n")]);
    let manifest = standard_manifest(repository.path());
    let case = case_with_indices(0, 0);
    let fingerprints = BenchmarkFingerprints {
        workloads: identity_fingerprints().workloads,
        cases: Vec::new(),
    };

    let error = fingerprints
        .identity_for(&manifest, &case)
        .expect_err("a missing measurement fingerprint must fail");

    assert!(matches!(
        error,
        BenchmarkIdentityError::MissingMeasurementFingerprint { .. }
    ));
}

#[test]
fn cli_frontend_and_profile_paths_receive_one_identity() {
    let repository = repository_with_files(&[("project/main.moth", b"value = 1\n")]);
    let manifest = standard_manifest(repository.path());
    let case = case_with_indices(0, 0);
    let fingerprints = identity_fingerprints();

    let cli_identity = fingerprints
        .identity_for(&manifest, &case)
        .expect("CLI identity should construct");
    let frontend_identity = fingerprints
        .identity_for(&manifest, &case)
        .expect("frontend identity should construct");
    let profile_manifest_identity = Some(
        fingerprints
            .identity_for(&manifest, &case)
            .expect("profile identity should construct"),
    );

    assert_eq!(cli_identity, frontend_identity);
    assert_eq!(Some(cli_identity), profile_manifest_identity);
}

#[test]
fn directory_enumeration_order_does_not_change_fingerprint() {
    let first = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/a.moth", b"a"),
        ("project/b.moth", b"b"),
    ]);
    let second = repository_with_files(&[
        ("project/b.moth", b"b"),
        ("project/a.moth", b"a"),
        ("project/main.moth", b"main"),
    ]);

    assert_eq!(
        source_fingerprint(&standard_manifest(first.path())),
        source_fingerprint(&standard_manifest(second.path()))
    );
}

#[test]
fn changing_file_bytes_changes_fingerprint() {
    let repository = repository_with_files(&[("project/main.moth", b"before")]);
    let manifest = standard_manifest(repository.path());
    let before = source_fingerprint(&manifest);

    write_file(repository.path(), "project/main.moth", b"after");

    assert_ne!(before, source_fingerprint(&manifest));
}

#[test]
fn renaming_file_changes_fingerprint() {
    let repository = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/old-name.moth", b"same bytes"),
    ]);
    let manifest = standard_manifest(repository.path());
    let before = source_fingerprint(&manifest);

    fs::rename(
        repository.path().join("project/old-name.moth"),
        repository.path().join("project/new-name.moth"),
    )
    .expect("test file should be renameable");

    assert_ne!(before, source_fingerprint(&manifest));
}

#[test]
fn runner_command_profile_and_args_change_fingerprint() {
    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let check = standard_manifest(repository.path());
    let build = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/dev"],
        vec![cli_runner(CliBenchmarkCommand::Build, &[])],
    );
    let check_with_args = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/dev"],
        vec![cli_runner(CliBenchmarkCommand::Check, &["--terse"])],
    );
    let frontend = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/dev"],
        vec![frontend_runner()],
    );

    let fingerprints = [
        measurement_fingerprint(&check),
        measurement_fingerprint(&build),
        measurement_fingerprint(&check_with_args),
        measurement_fingerprint(&frontend),
    ];
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        assert!(
            fingerprints[..index]
                .iter()
                .all(|previous| previous != fingerprint),
            "each complete runner declaration should have a distinct fingerprint"
        );
    }
}

#[test]
fn changed_runner_args_change_fingerprint_and_prevent_comparison() {
    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let previous_manifest = standard_manifest(repository.path());
    let current_manifest = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/dev"],
        vec![cli_runner(CliBenchmarkCommand::Check, &["--terse"])],
    );
    let shared_source_fingerprint = source_fingerprint(&previous_manifest).to_string();
    let previous_measurement_fingerprint = measurement_fingerprint(&previous_manifest).to_string();
    let current_measurement_fingerprint = measurement_fingerprint(&current_manifest).to_string();
    assert_ne!(
        current_measurement_fingerprint,
        previous_measurement_fingerprint
    );

    let make_result = |runner: BenchmarkRunner,
                       source_fp: String,
                       measurement_fp: String,
                       mean_ms: f64| BenchmarkCaseResult {
        case_id: "stable_case".to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: "workload".to_string(),
            source_fingerprint: source_fp,
            measurement_fingerprint: measurement_fp,
        }),
        group_name: "test".to_string(),
        runner,
        mean_ms,
        median_ms: mean_ms,
        stddev_ms: 0.0,
        observations: BenchmarkCaseObservations::default(),
    };
    let current = vec![make_result(
        current_manifest.cases[0].runner.clone(),
        shared_source_fingerprint.clone(),
        current_measurement_fingerprint,
        80.0,
    )];
    let previous = vec![make_result(
        previous_manifest.cases[0].runner.clone(),
        shared_source_fingerprint,
        previous_measurement_fingerprint,
        100.0,
    )];

    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.compared_case_count, 0);
    assert_eq!(comparison.workload_changed_case_ids.len(), 0);
    assert_eq!(comparison.measurement_changed_case_ids, ["stable_case"]);
    assert_eq!(comparison.overall_mean_delta_ms, None);
}

#[test]
fn every_case_measurement_fingerprint_is_isolated_from_siblings() {
    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let check_then_build = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &[],
        vec![
            cli_runner(CliBenchmarkCommand::Check, &[]),
            cli_runner(CliBenchmarkCommand::Build, &[]),
        ],
    );
    let build_then_check = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &[],
        vec![
            cli_runner(CliBenchmarkCommand::Build, &[]),
            cli_runner(CliBenchmarkCommand::Check, &[]),
        ],
    );
    let check_only = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &[],
        vec![cli_runner(CliBenchmarkCommand::Check, &[])],
    );

    // Swapping runner order changes every case's measurement fingerprint.
    assert_ne!(
        measurement_fingerprint(&check_then_build),
        measurement_fingerprint(&build_then_check)
    );
    // Case 0 with the same runner and source has the same measurement
    // fingerprint regardless of how many sibling cases exist.
    assert_eq!(
        measurement_fingerprint(&check_then_build),
        measurement_fingerprint(&check_only)
    );
}

#[test]
fn entry_and_ordered_root_exclude_declarations_change_fingerprint() {
    let repository = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/helper.moth", b"helper"),
    ]);
    let runner = || vec![cli_runner(CliBenchmarkCommand::Check, &[])];
    let base = manifest(
        repository.path(),
        "project/main.moth",
        &["project/main.moth", "project/helper.moth"],
        &[],
        runner(),
    );
    let changed_entry = manifest(
        repository.path(),
        "project/helper.moth",
        &["project/main.moth", "project/helper.moth"],
        &[],
        runner(),
    );
    let reversed_roots = manifest(
        repository.path(),
        "project/main.moth",
        &["project/helper.moth", "project/main.moth"],
        &[],
        runner(),
    );
    let excludes = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/dev", "project/release"],
        runner(),
    );
    let reversed_excludes = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/release", "project/dev"],
        runner(),
    );

    assert_ne!(
        source_fingerprint(&base),
        source_fingerprint(&changed_entry)
    );
    assert_ne!(
        source_fingerprint(&base),
        source_fingerprint(&reversed_roots)
    );
    assert_ne!(
        source_fingerprint(&excludes),
        source_fingerprint(&reversed_excludes)
    );
}

#[test]
fn length_prefixed_arguments_avoid_concatenation_ambiguity() {
    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let first = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &[],
        vec![cli_runner(CliBenchmarkCommand::Check, &["ab", "c"])],
    );
    let second = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &[],
        vec![cli_runner(CliBenchmarkCommand::Check, &["a", "bc"])],
    );

    assert_ne!(
        measurement_fingerprint(&first),
        measurement_fingerprint(&second)
    );
}

#[test]
fn excluded_output_changes_do_not_change_fingerprint() {
    let repository = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/dev/output.html", b"before"),
    ]);
    let manifest = standard_manifest(repository.path());
    let before = source_fingerprint(&manifest);

    write_file(repository.path(), "project/dev/output.html", b"after");
    write_file(repository.path(), "project/dev/nested/new.js", b"new");

    assert_eq!(before, source_fingerprint(&manifest));
}

#[test]
fn excludes_use_exact_component_prefixes() {
    let repository = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/dev/output.html", b"excluded"),
        ("project/dev-output/output.html", b"included before"),
    ]);
    let manifest = standard_manifest(repository.path());
    let before = source_fingerprint(&manifest);

    write_file(
        repository.path(),
        "project/dev-output/output.html",
        b"included after",
    );

    assert_ne!(before, source_fingerprint(&manifest));
}

#[test]
fn adding_an_included_file_changes_fingerprint() {
    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let manifest = standard_manifest(repository.path());
    let before = source_fingerprint(&manifest);

    write_file(repository.path(), "project/added.moth", b"added");

    assert_ne!(before, source_fingerprint(&manifest));
}

#[test]
fn missing_root_returns_contextual_typed_error() {
    let repository = tempdir().expect("temporary repository should exist");
    let manifest = manifest(
        repository.path(),
        "missing.moth",
        &["missing.moth"],
        &[],
        vec![cli_runner(CliBenchmarkCommand::Check, &[])],
    );

    let error = compute_benchmark_fingerprints(&manifest)
        .expect_err("missing fingerprint root should fail");

    assert!(matches!(
        error,
        BenchmarkFingerprintError::RootAccess {
            workload_id,
            path,
            source,
        } if workload_id == "workload"
            && path == Path::new("missing.moth")
            && source.kind() == io::ErrorKind::NotFound
    ));
}

#[test]
fn fully_excluded_file_set_fails() {
    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let manifest = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/main.moth"],
        vec![cli_runner(CliBenchmarkCommand::Check, &[])],
    );

    let error = compute_benchmark_fingerprints(&manifest)
        .expect_err("a workload with no included files should fail");

    assert!(matches!(
        error,
        BenchmarkFingerprintError::EmptyFileSet { workload_id }
            if workload_id == "workload"
    ));
}

#[test]
fn repository_relative_path_escape_fails() {
    let parent = tempdir().expect("temporary parent should exist");
    let repository_root = parent.path().join("repository");
    fs::create_dir(&repository_root).expect("repository directory should be creatable");
    write_file(parent.path(), "outside.moth", b"outside");
    let manifest = manifest(
        &repository_root,
        "../outside.moth",
        &["../outside.moth"],
        &[],
        vec![cli_runner(CliBenchmarkCommand::Check, &[])],
    );

    let error =
        compute_benchmark_fingerprints(&manifest).expect_err("repository path escape should fail");

    assert!(matches!(
        error,
        BenchmarkFingerprintError::InvalidLogicalPath { workload_id, .. }
            if workload_id == "workload"
    ));
}

#[cfg(unix)]
#[test]
fn symlink_escape_fails() {
    use std::os::unix::fs::symlink;

    let repository = repository_with_files(&[("project/main.moth", b"main")]);
    let outside = repository_with_files(&[("outside.moth", b"outside")]);
    symlink(
        outside.path().join("outside.moth"),
        repository.path().join("project/escape.moth"),
    )
    .expect("test symlink should be creatable");

    let error = compute_benchmark_fingerprints(&standard_manifest(repository.path()))
        .expect_err("symlink escape should fail");

    assert!(matches!(
        error,
        BenchmarkFingerprintError::Symlink { workload_id, path }
            if workload_id == "workload" && path == Path::new("project/escape.moth")
    ));
}

#[cfg(unix)]
#[test]
fn in_repository_symlink_fails() {
    use std::os::unix::fs::symlink;

    let repository = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/target.moth", b"target"),
    ]);
    symlink(
        repository.path().join("project/target.moth"),
        repository.path().join("project/alias.moth"),
    )
    .expect("test symlink should be creatable");

    let error = compute_benchmark_fingerprints(&standard_manifest(repository.path()))
        .expect_err("in-repository symlink should fail");

    assert!(matches!(
        error,
        BenchmarkFingerprintError::Symlink { workload_id, path }
            if workload_id == "workload" && path == Path::new("project/alias.moth")
    ));
}

#[cfg(unix)]
#[test]
fn symlink_inside_excluded_subtree_is_ignored() {
    use std::os::unix::fs::symlink;

    let repository = repository_with_files(&[
        ("project/main.moth", b"main"),
        ("project/target.moth", b"target"),
        ("project/dev/nested/output.html", b"excluded"),
    ]);
    symlink(
        repository.path().join("project/target.moth"),
        repository.path().join("project/dev/nested/alias.moth"),
    )
    .expect("test symlink should be creatable");

    let manifest = standard_manifest(repository.path());
    let with_excluded_symlink = source_fingerprint(&manifest);

    fs::remove_file(repository.path().join("project/dev/nested/alias.moth"))
        .expect("test symlink should be removable");

    assert_eq!(with_excluded_symlink, source_fingerprint(&manifest));
}

#[test]
fn bulk_api_preserves_manifest_workload_order() {
    let repository = repository_with_files(&[("first.moth", b"first"), ("second.moth", b"second")]);
    let first_runner = cli_runner(CliBenchmarkCommand::Check, &[]);
    let second_runner = frontend_runner();
    let bulk_manifest = BenchmarkManifest {
        workloads: vec![
            BenchmarkWorkload {
                id: "first".to_owned(),
                entry: "first.moth".into(),
                entry_kind: BenchmarkEntryKind::File,
                fingerprint_mode: BenchmarkFingerprintMode::FullTree,
                fingerprint_roots: vec!["first.moth".into()],
                fingerprint_excludes: vec![],
                generated_output_roots: Vec::new(),
            },
            BenchmarkWorkload {
                id: "second".to_owned(),
                entry: "second.moth".into(),
                entry_kind: BenchmarkEntryKind::File,
                fingerprint_mode: BenchmarkFingerprintMode::FullTree,
                fingerprint_roots: vec!["second.moth".into()],
                fingerprint_excludes: vec![],
                generated_output_roots: Vec::new(),
            },
        ],
        cases: vec![
            BenchmarkCase {
                id: "second_case".to_owned(),
                case_index: 0,
                workload_index: 1,
                group_name: BenchmarkGroup::Core,
                quick: false,
                expectation: BenchmarkExpectation::Clean,
                runner: second_runner.clone(),
            },
            BenchmarkCase {
                id: "first_case".to_owned(),
                case_index: 1,
                workload_index: 0,
                group_name: BenchmarkGroup::Core,
                quick: false,
                expectation: BenchmarkExpectation::Clean,
                runner: first_runner.clone(),
            },
        ],
        manifest_path: repository.path().join("benchmarks/manifest.toml"),
        repository_root: repository.path().to_owned(),
    };
    let fingerprints =
        compute_benchmark_fingerprints(&bulk_manifest).expect("bulk fingerprints should compute");
    let first_only = manifest(
        repository.path(),
        "first.moth",
        &["first.moth"],
        &[],
        vec![first_runner],
    );
    let second_only = manifest(
        repository.path(),
        "second.moth",
        &["second.moth"],
        &[],
        vec![second_runner],
    );

    assert_eq!(
        fingerprints.workloads,
        [
            source_fingerprint(&first_only),
            source_fingerprint(&second_only)
        ]
    );
}

#[test]
fn versioned_fingerprint_has_stable_hex_encoding() {
    let repository = repository_with_files(&[
        ("project/main.moth", b"main\n"),
        ("project/helper.moth", b"helper\n"),
    ]);
    let manifest = manifest(
        repository.path(),
        "project/main.moth",
        &["project"],
        &["project/dev"],
        vec![
            cli_runner(CliBenchmarkCommand::Check, &["--terse"]),
            frontend_runner(),
        ],
    );

    assert_eq!(
        source_fingerprint(&manifest).to_string(),
        "f4fb277385e7758c6a24f95ddc645637"
    );
}
