//! Tests for profile artifact layout and file writing.

use super::*;
use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric, GitRevision};
use crate::profile::observations::ProfileObservation;
use crate::profile::options::ProfileFilterMode;
use crate::profile::parse::ProfileShapeDump;
use std::path::Path;

/// Build a test observation with sample data.
fn test_observation() -> ProfileObservation {
    ProfileObservation {
        case_id: "test_case_bst".to_string(),
        group_name: "stress".to_string(),
        command: "check".to_string(),
        command_args: vec!["test.moth".to_string()],
        wall_ms: 1234.5,
        observations: BenchmarkCaseObservations {
            stage_timings: vec![
                BenchmarkMetric {
                    name: "ast_ms".to_string(),
                    value: 812.0,
                },
                BenchmarkMetric {
                    name: "headers_ms".to_string(),
                    value: 200.0,
                },
            ],
            counters: vec![BenchmarkMetric {
                name: "token_count".to_string(),
                value: 12000.0,
            }],
        },
        stdout: "mock stdout".to_string(),
        stderr: "mock stderr".to_string(),
    }
}

#[test]
fn profile_case_paths_use_authored_case_id_verbatim() {
    let run = ProfileRunPaths {
        run_id: "2026-06-18T10-30-abc1234".to_string(),
        root: Path::new("/tmp/test-run").into(),
    };

    let case = run.case_paths("authored_case_7");
    assert!(case.case_dir.ends_with("cases/authored_case_7"));
    assert!(case.stdout_log.to_str().unwrap().ends_with("stdout.log"));
    assert!(case.stderr_log.to_str().unwrap().ends_with("stderr.log"));
    assert!(
        case.observations_json
            .to_str()
            .unwrap()
            .ends_with("detailed-observations.json")
    );
    assert!(case.summary_md.to_str().unwrap().ends_with("summary.md"));
    assert!(
        case.profile_json
            .to_str()
            .unwrap()
            .ends_with("profile.json.gz")
    );
    assert!(
        case.profile_shape_txt
            .to_str()
            .unwrap()
            .ends_with("profile-shape.txt")
    );
}

#[test]
fn profile_run_paths_manifest_and_index_are_in_root() {
    let run = ProfileRunPaths {
        run_id: "2026-06-18T10-30-abc1234".to_string(),
        root: Path::new("/tmp/test-run").into(),
    };

    assert!(
        run.manifest_path()
            .to_str()
            .unwrap()
            .ends_with("run-manifest.json")
    );
    assert!(run.index_path().to_str().unwrap().ends_with("index.md"));
}

#[test]
fn filter_label_returns_correct_strings() {
    // Tested indirectly through formatting, but verify the mapping.
    let git_revision = GitRevision {
        commit: Some("abc1234".to_string()),
        dirty: None,
    };
    let manifest = format_run_manifest_json(
        "test-run",
        Some(&git_revision),
        ProfileFilterMode::Terse,
        None,
        &[],
    );
    assert!(manifest.contains(r#""filter": "terse""#));
}

#[test]
fn display_label_returns_correct_strings() {
    assert_eq!(ProfileFilterMode::Terse.display_label(), "terse");
    assert_eq!(ProfileFilterMode::Normal.display_label(), "normal");
    assert_eq!(ProfileFilterMode::Deep.display_label(), "deep");
    assert_eq!(ProfileFilterMode::RawIndex.display_label(), "raw-index");
}

#[test]
fn format_run_manifest_json_with_empty_cases() {
    let git_revision = GitRevision {
        commit: Some("abc1234".to_string()),
        dirty: None,
    };
    let json = format_run_manifest_json(
        "2026-06-18T10-30-abc1234",
        Some(&git_revision),
        ProfileFilterMode::Normal,
        Some(500.0),
        &[],
    );

    assert!(json.contains(r#""format_version": 3"#));
    assert!(json.contains(r#""run_id": "2026-06-18T10-30-abc1234""#));
    assert!(json.contains(r#""commit": "abc1234""#));
    assert!(json.contains(r#""filter": "normal""#));
    assert!(json.contains(r#""samply_rate_hz": 500"#));
    // Empty cases array spans multiple lines in manual JSON.
    assert!(json.contains(r#""cases": ["#));
}

#[test]
fn format_run_manifest_json_with_null_commit() {
    let json = format_run_manifest_json("test-run", None, ProfileFilterMode::Terse, None, &[]);
    assert!(json.contains(r#""commit": null"#));
    assert!(json.contains(r#""git_dirty": null"#));
    assert!(json.contains(r#""samply_rate_hz": null"#));
}

#[test]
fn format_run_manifest_json_with_cases() {
    let cases = vec![ProfileCaseManifest {
        case_id: "check_foo_bst".to_string(),
        identity: None,
        group_name: "core".to_string(),
        command: "check".to_string(),
        args: vec!["foo.moth".to_string()],
        observation_wall_ms: 500.0,
        profile_path: "cases/check_foo_bst/profile.json.gz".to_string(),
        stdout_path: "cases/check_foo_bst/stdout.log".to_string(),
        stderr_path: "cases/check_foo_bst/stderr.log".to_string(),
        summary_path: Some("cases/check_foo_bst/summary.md".to_string()),
    }];

    let git_revision = GitRevision {
        commit: Some("abc".to_string()),
        dirty: None,
    };
    let json = format_run_manifest_json(
        "test-run",
        Some(&git_revision),
        ProfileFilterMode::Deep,
        None,
        &cases,
    );

    assert!(json.contains(r#""case_id": "check_foo_bst""#));
    assert!(!json.contains(r#""case_name""#));
    assert!(json.contains(r#""group_name": "core""#));
    assert!(json.contains(r#""observation_wall_ms": 500"#));
    assert!(json.contains(r#""filter": "deep""#));
}

#[test]
fn format_run_manifest_json_preserves_identity_and_optional_summary_paths() {
    let cases = vec![
        ProfileCaseManifest {
            case_id: "with_identity".to_string(),
            identity: Some(crate::bench_types::BenchmarkMeasurementIdentity {
                workload_id: "workload".to_string(),
                source_fingerprint: "source".to_string(),
                measurement_fingerprint: "measurement".to_string(),
            }),
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: Vec::new(),
            observation_wall_ms: 1.0,
            profile_path: "cases/with_identity/profile.json.gz".to_string(),
            stdout_path: "cases/with_identity/stdout.log".to_string(),
            stderr_path: "cases/with_identity/stderr.log".to_string(),
            summary_path: Some("cases/with_identity/summary.md".to_string()),
        },
        ProfileCaseManifest {
            case_id: "raw_index".to_string(),
            identity: None,
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: Vec::new(),
            observation_wall_ms: 2.0,
            profile_path: "cases/raw_index/profile.json.gz".to_string(),
            stdout_path: "cases/raw_index/stdout.log".to_string(),
            stderr_path: "cases/raw_index/stderr.log".to_string(),
            summary_path: None,
        },
    ];

    let json =
        format_run_manifest_json("test-run", None, ProfileFilterMode::RawIndex, None, &cases);
    let document: serde_json::Value =
        serde_json::from_str(&json).expect("run manifest should be valid JSON");
    let entries = document["cases"]
        .as_array()
        .expect("run manifest cases should be an array");

    assert_eq!(entries[0]["workload_id"], "workload");
    assert_eq!(entries[0]["source_fingerprint"], "source");
    assert_eq!(entries[0]["measurement_fingerprint"], "measurement");
    assert_eq!(entries[0]["summary_path"], "cases/with_identity/summary.md");
    assert!(entries[1]["workload_id"].is_null());
    assert!(entries[1]["source_fingerprint"].is_null());
    assert!(entries[1]["measurement_fingerprint"].is_null());
    assert!(entries[1]["summary_path"].is_null());
}

#[test]
fn raw_index_manifest_only_advertises_written_artifacts() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");
    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");
    let case_paths = run_paths.case_paths("raw_index_case");
    case_paths.create_dir().expect("create case dir");
    for path in [
        &case_paths.profile_json,
        &case_paths.stdout_log,
        &case_paths.stderr_log,
    ] {
        std::fs::write(path, "written").expect("write raw-index artifact");
    }

    let cases = vec![ProfileCaseManifest {
        case_id: "raw_index_case".to_string(),
        identity: None,
        group_name: "core".to_string(),
        command: "check".to_string(),
        args: Vec::new(),
        observation_wall_ms: 1.0,
        profile_path: "cases/raw_index_case/profile.json.gz".to_string(),
        stdout_path: "cases/raw_index_case/stdout.log".to_string(),
        stderr_path: "cases/raw_index_case/stderr.log".to_string(),
        summary_path: None,
    }];
    write_run_manifest(
        &run_paths,
        "test-run",
        None,
        ProfileFilterMode::RawIndex,
        None,
        &cases,
    )
    .expect("write raw-index manifest");

    let content = std::fs::read_to_string(run_paths.manifest_path()).expect("read manifest");
    let document: serde_json::Value =
        serde_json::from_str(&content).expect("raw-index manifest should be valid JSON");
    let entry = &document["cases"][0];
    for field in ["profile_path", "stdout_path", "stderr_path"] {
        let relative_path = entry[field]
            .as_str()
            .expect("advertised artifact path should be a string");
        assert!(
            run_paths.root.join(relative_path).is_file(),
            "advertised {field} should exist"
        );
    }
    assert!(entry["summary_path"].is_null());
    assert!(
        !run_paths
            .root
            .join("cases/raw_index_case/summary.md")
            .exists()
    );
}

#[test]
fn format_observations_json_matches_plan_schema() {
    let observation = test_observation();
    let json = format_observations_json(&observation);

    // Must contain the expected top-level keys.
    assert!(json.contains(r#""format_version": 2"#));
    assert!(json.contains(r#""case_id": "test_case_bst""#));
    assert!(!json.contains(r#""case_name""#));
    assert!(json.contains(r#""group": "stress""#));
    assert!(json.contains(r#""wall_ms": 1234.5"#));
    // Command array uses no space after comma in manual JSON.
    assert!(json.contains(r#""command": ["check","test.moth"]"#));

    // Stage timings and counters must be present.
    assert!(json.contains(r#""name": "ast_ms""#));
    assert!(json.contains(r#""value": 812"#));
    assert!(json.contains(r#""name": "token_count""#));
    assert!(json.contains(r#""value": 12000"#));
}

#[test]
fn format_index_md_lists_cases() {
    let cases = vec![ProfileCaseManifest {
        case_id: "check_foo_bst".to_string(),
        identity: None,
        group_name: "core".to_string(),
        command: "check".to_string(),
        args: vec!["foo.moth".to_string()],
        observation_wall_ms: 500.0,
        profile_path: "profile.json.gz".to_string(),
        stdout_path: "stdout.log".to_string(),
        stderr_path: "stderr.log".to_string(),
        summary_path: Some("summary.md".to_string()),
    }];

    let md = format_index_md("test-run", ProfileFilterMode::Terse, &cases);

    assert!(md.contains("# Profiling run: test-run"));
    assert!(md.contains("Cases: 1"));
    assert!(md.contains("check_foo_bst"));
    assert!(md.contains("~500ms"));
}

#[test]
fn format_index_md_empty_cases() {
    let md = format_index_md("test-run", ProfileFilterMode::RawIndex, &[]);
    assert!(md.contains("Cases: 0"));
}

#[test]
fn format_metric_array_json_empty() {
    let result = format_metric_array_json(&[]);
    assert_eq!(result, "[]");
}

#[test]
fn format_metric_array_json_single_item() {
    let metrics = vec![BenchmarkMetric {
        name: "test_metric".to_string(),
        value: 42.0,
    }];
    let result = format_metric_array_json(&metrics);
    assert!(result.contains(r#""name": "test_metric""#));
    assert!(result.contains(r#""value": 42"#));
}

#[test]
fn format_profile_shape_dump_lists_symbolication_diagnostics() {
    let shape = ProfileShapeDump {
        meta_product: "samply".to_string(),
        meta_version: "0.13.1".to_string(),
        thread_count: 1,
        first_thread_func_table_keys: vec!["name".to_string(), "resource".to_string()],
        first_20_func_names: vec!["0x1000".to_string(), "moth::ast::emit".to_string()],
        resource_table_keys: vec!["lib".to_string()],
        libs_count: Some(2),
        first_10_libs: vec!["moth".to_string(), "libsystem_kernel.dylib".to_string()],
        native_symbols_present: true,
    };

    let text = format_profile_shape_dump(&shape);

    assert!(text.contains("meta.product: samply"));
    assert!(text.contains("meta.version: 0.13.1"));
    assert!(text.contains("threads: 1"));
    assert!(text.contains("first thread funcTable keys: name, resource"));
    assert!(text.contains("  - 0x1000"));
    assert!(text.contains("  - moth::ast::emit"));
    assert!(text.contains("resourceTable keys: lib"));
    assert!(text.contains("libs count: 2"));
    assert!(text.contains("nativeSymbols present: yes"));
}

#[test]
fn create_run_paths_in_temp_directory() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");

    assert!(run_paths.root.exists());
    assert!(run_paths.root.join("cases").exists());
    assert!(run_paths.run_id.contains("abc1234"));
}

#[test]
fn create_case_paths_directory() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");
    let case_paths = run_paths.case_paths("test_case");

    case_paths.create_dir().expect("create case dir");
    assert!(case_paths.case_dir.exists());
}

#[test]
fn write_and_read_stdout_stderr_logs() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");
    let case_paths = run_paths.case_paths("test_case");
    case_paths.create_dir().expect("create case dir");

    case_paths
        .write_stdout("hello stdout")
        .expect("write stdout");
    case_paths
        .write_stderr("hello stderr")
        .expect("write stderr");

    let stdout = std::fs::read_to_string(&case_paths.stdout_log).expect("read stdout");
    let stderr = std::fs::read_to_string(&case_paths.stderr_log).expect("read stderr");

    assert_eq!(stdout, "hello stdout");
    assert_eq!(stderr, "hello stderr");
}

#[test]
fn write_observations_json_creates_valid_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");
    let case_paths = run_paths.case_paths("test_case");
    case_paths.create_dir().expect("create case dir");

    let observation = test_observation();
    case_paths
        .write_observations_json(&observation)
        .expect("write observations");

    let content =
        std::fs::read_to_string(&case_paths.observations_json).expect("read observations");
    assert!(content.contains(r#""format_version": 2"#));
    assert!(content.contains(r#""case_id": "test_case_bst""#));
}

#[test]
fn write_profile_shape_dump_creates_diagnostic_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");
    let case_paths = run_paths.case_paths("test_case");
    case_paths.create_dir().expect("create case dir");

    let shape = ProfileShapeDump {
        meta_product: "samply".to_string(),
        meta_version: "0.13.1".to_string(),
        thread_count: 0,
        first_thread_func_table_keys: Vec::new(),
        first_20_func_names: Vec::new(),
        resource_table_keys: Vec::new(),
        libs_count: None,
        first_10_libs: Vec::new(),
        native_symbols_present: false,
    };

    write_profile_shape_dump(&case_paths, &shape).expect("write profile shape dump");

    let content =
        std::fs::read_to_string(&case_paths.profile_shape_txt).expect("read profile shape");
    assert!(content.contains("first thread funcTable keys: none"));
    assert!(content.contains("nativeSymbols present: no"));
}

#[test]
fn write_run_manifest_creates_valid_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");

    let cases = vec![ProfileCaseManifest {
        case_id: "test_case".to_string(),
        identity: None,
        group_name: "core".to_string(),
        command: "check".to_string(),
        args: vec!["foo.moth".to_string()],
        observation_wall_ms: 100.0,
        profile_path: "cases/test_case/profile.json.gz".to_string(),
        stdout_path: "cases/test_case/stdout.log".to_string(),
        stderr_path: "cases/test_case/stderr.log".to_string(),
        summary_path: Some("cases/test_case/summary.md".to_string()),
    }];

    let git_revision = GitRevision {
        commit: Some("abc1234".to_string()),
        dirty: None,
    };
    write_run_manifest(
        &run_paths,
        "test-run",
        Some(&git_revision),
        ProfileFilterMode::Terse,
        None,
        &cases,
    )
    .expect("write manifest");

    let content = std::fs::read_to_string(run_paths.manifest_path()).expect("read manifest");
    let _: serde_json::Value =
        serde_json::from_str(&content).expect("written manifest should be valid JSON");
    assert!(content.contains(r#""format_version": 3"#));
    assert!(content.contains(r#""case_id": "test_case""#));
}

#[test]
fn write_index_md_creates_valid_file() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let profiles_root = temp_dir.path().join("profiles");

    let run_paths =
        ProfileRunPaths::create(&profiles_root, Some("abc1234")).expect("create run paths");

    write_index_md(&run_paths, "test-run", ProfileFilterMode::Terse, &[]).expect("write index");

    let content = std::fs::read_to_string(run_paths.index_path()).expect("read index");
    assert!(content.contains("# Profiling run: test-run"));
}
