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
            timing_schema_version: 2,
            stage_timings: vec![
                BenchmarkMetric {
                    name: "frontend.ast.total".to_string(),
                    value: 812.0,
                },
                BenchmarkMetric {
                    name: "frontend.bind_headers".to_string(),
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

/// Build a test case manifest entry with a fixed identity.
fn test_case_manifest(case_id: &str, summary_path: Option<String>) -> ProfileCaseManifest {
    ProfileCaseManifest {
        case_id: case_id.to_string(),
        identity: crate::bench_types::BenchmarkMeasurementIdentity {
            workload_id: "workload".to_string(),
            source_fingerprint: "source".to_string(),
            measurement_fingerprint: "measurement".to_string(),
            timing_schema_version: 2,
        },
        group_name: "core".to_string(),
        command: "check".to_string(),
        args: vec!["foo.moth".to_string()],
        observation_wall_ms: 1.0,
        profile_path: format!("cases/{case_id}/profile.json.gz"),
        stdout_path: format!("cases/{case_id}/stdout.log"),
        stderr_path: format!("cases/{case_id}/stderr.log"),
        summary_path,
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
        dirty: Some(false),
    };
    let manifest = RunManifestFile::new(
        "test-run",
        Some(&git_revision),
        ProfileFilterMode::Terse,
        None,
        &[],
    );
    let json = serde_json::to_string_pretty(&manifest).expect("manifest should serialize");
    assert!(json.contains(r#""filter": "terse""#));
}

#[test]
fn display_label_returns_correct_strings() {
    assert_eq!(ProfileFilterMode::Terse.display_label(), "terse");
    assert_eq!(ProfileFilterMode::Normal.display_label(), "normal");
    assert_eq!(ProfileFilterMode::Deep.display_label(), "deep");
    assert_eq!(ProfileFilterMode::RawIndex.display_label(), "raw-index");
}

#[test]
fn run_manifest_file_with_empty_cases() {
    let git_revision = GitRevision {
        commit: Some("abc1234".to_string()),
        dirty: Some(false),
    };
    let manifest = RunManifestFile::new(
        "2026-06-18T10-30-abc1234",
        Some(&git_revision),
        ProfileFilterMode::Normal,
        Some(500.0),
        &[],
    );
    let json = serde_json::to_string_pretty(&manifest).expect("manifest should serialize");

    assert!(json.contains(r#""format_version": 4"#));
    assert!(json.contains(r#""run_id": "2026-06-18T10-30-abc1234""#));
    assert!(json.contains(r#""commit": "abc1234""#));
    assert!(json.contains(r#""filter": "normal""#));
    assert!(json.contains(r#""samply_rate_hz": 500"#));
}

#[test]
fn run_manifest_file_with_null_revision_fields() {
    let manifest = RunManifestFile::new("test-run", None, ProfileFilterMode::Terse, None, &[]);
    let json = serde_json::to_string_pretty(&manifest).expect("manifest should serialize");

    assert!(json.contains(r#""commit": null"#));
    assert!(json.contains(r#""git_dirty": null"#));
    assert!(json.contains(r#""samply_rate_hz": null"#));
}

#[test]
fn run_manifest_file_with_cases() {
    let mut case = test_case_manifest("check_foo_bst", None);
    case.observation_wall_ms = 500.0;
    case.args = vec!["foo.moth".to_string()];
    let cases = vec![case];

    let git_revision = GitRevision {
        commit: Some("abc".to_string()),
        dirty: Some(false),
    };
    let manifest = RunManifestFile::new(
        "test-run",
        Some(&git_revision),
        ProfileFilterMode::Deep,
        None,
        &cases,
    );
    let json = serde_json::to_string_pretty(&manifest).expect("manifest should serialize");

    assert!(json.contains(r#""case_id": "check_foo_bst""#));
    assert!(!json.contains(r#""case_name""#));
    assert!(json.contains(r#""group_name": "core""#));
    assert!(json.contains(r#""observation_wall_ms": 500"#));
    assert!(json.contains(r#""filter": "deep""#));
}

#[test]
fn run_manifest_file_preserves_identity_and_optional_summary_paths() {
    let mut with_summary = test_case_manifest("with_identity", None);
    with_summary.summary_path = Some("cases/with_identity/summary.md".to_string());
    let cases = vec![with_summary, test_case_manifest("raw_index", None)];

    let manifest =
        RunManifestFile::new("test-run", None, ProfileFilterMode::RawIndex, None, &cases);
    let json = serde_json::to_string_pretty(&manifest).expect("manifest should serialize");
    let document: serde_json::Value =
        serde_json::from_str(&json).expect("run manifest should be valid JSON");
    let entries = document["cases"]
        .as_array()
        .expect("run manifest cases should be an array");

    assert_eq!(entries[0]["workload_id"], "workload");
    assert_eq!(entries[0]["source_fingerprint"], "source");
    assert_eq!(entries[0]["measurement_fingerprint"], "measurement");
    assert_eq!(entries[0]["summary_path"], "cases/with_identity/summary.md");
    assert_eq!(entries[1]["workload_id"], "workload");
    assert_eq!(entries[1]["source_fingerprint"], "source");
    assert_eq!(entries[1]["measurement_fingerprint"], "measurement");
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

    let mut case = test_case_manifest("raw_index_case", None);
    case.args = Vec::new();
    case.observation_wall_ms = 1.0;
    case.profile_path = "cases/raw_index_case/profile.json.gz".to_string();
    case.stdout_path = "cases/raw_index_case/stdout.log".to_string();
    case.stderr_path = "cases/raw_index_case/stderr.log".to_string();
    let cases = vec![case];
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
fn detailed_observations_file_matches_plan_schema() {
    let observation = test_observation();
    let json = serde_json::to_string_pretty(&DetailedObservationsFile::from(&observation))
        .expect("observations should serialize");

    assert!(json.contains(r#""format_version": 2"#));
    assert!(json.contains(r#""case_id": "test_case_bst""#));
    assert!(!json.contains(r#""case_name""#));
    assert!(json.contains(r#""group": "stress""#));
    assert!(json.contains(r#""wall_ms": 1234.5"#));
    assert!(json.contains(r#""check"#));
    assert!(json.contains(r#""test.moth""#));
    assert!(json.contains(r#""name": "frontend.ast.total""#));
    assert!(json.contains(r#""value": 812"#));
    assert!(json.contains(r#""name": "token_count""#));
    assert!(json.contains(r#""value": 12000"#));
}

#[test]
fn format_index_md_lists_cases() {
    let mut case = test_case_manifest("check_foo_bst", None);
    case.observation_wall_ms = 500.0;
    case.profile_path = "profile.json.gz".to_string();
    case.stdout_path = "stdout.log".to_string();
    let cases = vec![case];

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

    let mut case = test_case_manifest("test_case", None);
    case.observation_wall_ms = 100.0;
    case.summary_path = Some("cases/test_case/summary.md".to_string());
    let cases = vec![case];

    let git_revision = GitRevision {
        commit: Some("abc1234".to_string()),
        dirty: Some(false),
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
    assert!(content.contains(r#""format_version": 4"#));
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
