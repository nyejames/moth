//! Tests for profile history storage and retrieval.

use super::*;
use crate::bench_types::BenchmarkMetric;

/// Build a test history record with one case.
fn test_record(run_id: &str) -> ProfileHistoryRecord {
    ProfileHistoryRecord {
        format_version: HISTORY_FORMAT_VERSION,
        run_id: run_id.to_string(),
        timestamp: "June 18th - 10:30".to_string(),
        commit: Some("abc1234".to_string()),
        system_uuid: "TEST-UUID-001".to_string(),
        system_display: "Test System".to_string(),
        filter_mode: "terse".to_string(),
        sample_rate_hz: None,
        cases: vec![HistoryCaseRecord {
            case_id: "check_foo_bst".to_string(),
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: vec!["foo.moth".to_string()],
            observation_wall_ms: 1234.5,
            sample_count: 500,
            sample_weight: 500.0,
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 812.0,
            }],
            counters: vec![BenchmarkMetric {
                name: "token_count".to_string(),
                value: 12000.0,
            }],
            hot_functions: vec![HistoryHotFunction {
                name: "moth::compiler_frontend::ast::resolve_type".to_string(),
                bucket_label: "AST".to_string(),
                inclusive_samples: 150.0,
                self_samples: 80.0,
                inclusive_pct: 30.0,
                self_pct: 16.0,
            }],
            top_bucket_label: "AST".to_string(),
            run_directory_path: "benchmarks/local-data/profiles/2026-06-18T10-30-abc1234"
                .to_string(),
        }],
    }
}

/// Build a second test record with different data.
fn test_record_b(run_id: &str) -> ProfileHistoryRecord {
    ProfileHistoryRecord {
        format_version: HISTORY_FORMAT_VERSION,
        run_id: run_id.to_string(),
        timestamp: "June 18th - 11:00".to_string(),
        commit: Some("def5678".to_string()),
        system_uuid: "TEST-UUID-001".to_string(),
        system_display: "Test System".to_string(),
        filter_mode: "terse".to_string(),
        sample_rate_hz: Some(1000.0),
        cases: vec![HistoryCaseRecord {
            case_id: "check_foo_bst".to_string(),
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: vec!["foo.moth".to_string()],
            observation_wall_ms: 1400.0,
            sample_count: 600,
            sample_weight: 600.0,
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 900.0,
            }],
            counters: vec![BenchmarkMetric {
                name: "token_count".to_string(),
                value: 13000.0,
            }],
            hot_functions: vec![HistoryHotFunction {
                name: "moth::compiler_frontend::ast::resolve_type".to_string(),
                bucket_label: "AST".to_string(),
                inclusive_samples: 200.0,
                self_samples: 100.0,
                inclusive_pct: 33.3,
                self_pct: 16.7,
            }],
            top_bucket_label: "AST".to_string(),
            run_directory_path: "benchmarks/local-data/profiles/2026-06-18T11-00-def5678"
                .to_string(),
        }],
    }
}

#[test]
fn append_and_read_single_record() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
    assert_eq!(records[0].system_uuid, "TEST-UUID-001");
    assert_eq!(records[0].filter_mode, "terse");
    assert_eq!(records[0].cases.len(), 1);
    assert_eq!(records[0].cases[0].case_id, "check_foo_bst");
    assert_eq!(records[0].cases[0].sample_count, 500);
}

#[test]
fn append_multiple_records_and_read_all() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record_a = test_record("2026-06-18T10-30-abc1234");
    let record_b = test_record_b("2026-06-18T11-00-def5678");

    append_profile_run(&path, &record_a).expect("append a");
    append_profile_run(&path, &record_b).expect("append b");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
    assert_eq!(records[1].run_id, "2026-06-18T11-00-def5678");
    assert_eq!(records[1].sample_rate_hz, Some(1000.0));
}

#[test]
fn read_empty_file_returns_empty_vec() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let records = read_profile_runs(&path).expect("read");
    assert!(records.is_empty());
}

#[test]
fn read_missing_file_returns_empty_vec() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("nonexistent.jsonl");

    let records = read_profile_runs(&path).expect("read");
    assert!(records.is_empty());
}

#[test]
fn malformed_lines_are_skipped_with_warning() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    // Write a malformed line followed by a valid line.
    let record = test_record("2026-06-18T10-30-abc1234");
    let valid_line = format_record_as_jsonl(&record);
    let content = format!("this is not json\n{}\n", valid_line);
    std::fs::write(&path, content).expect("write");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
}

#[test]
fn unknown_format_version_is_skipped() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    let valid_line = format_record_as_jsonl(&record);
    // Write a line with a future format_version.
    let content = r#"{"format_version":999,"run_id":"future","timestamp":"now","system_uuid":"x","system_display":"x","filter_mode":"terse","cases":[]}"#;
    std::fs::write(&path, format!("{}\n{}\n", content, valid_line)).expect("write");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
}

#[test]
fn roundtrip_preserves_case_data() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    let case = &records[0].cases[0];

    assert_eq!(case.observation_wall_ms, 1234.5);
    assert_eq!(case.sample_count, 500);
    assert_eq!(case.sample_weight, 500.0);
    assert_eq!(case.stage_timings.len(), 1);
    assert_eq!(case.stage_timings[0].name, "ast_ms");
    assert_eq!(case.stage_timings[0].value, 812.0);
    assert_eq!(case.counters.len(), 1);
    assert_eq!(case.counters[0].name, "token_count");
    assert_eq!(case.counters[0].value, 12000.0);
    assert_eq!(case.hot_functions.len(), 1);
    assert_eq!(
        case.hot_functions[0].name,
        "moth::compiler_frontend::ast::resolve_type"
    );
    assert_eq!(case.hot_functions[0].bucket_label, "AST");
    assert_eq!(case.hot_functions[0].inclusive_pct, 30.0);
    assert_eq!(case.hot_functions[0].self_pct, 16.0);
    assert_eq!(case.top_bucket_label, "AST");
}

#[test]
fn legacy_v1_case_name_is_adapted_to_current_case_id() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let legacy = r#"{"format_version":1,"run_id":"legacy","timestamp":"June 18th - 09:00","commit":null,"system_uuid":"TEST-UUID-001","system_display":"Test System","filter_mode":"terse","sample_rate_hz":null,"cases":[{"case_name":"authored_case","group_name":"core","command":"check","args":["foo.moth"],"observation_wall_ms":10.0,"sample_count":2,"sample_weight":2.0,"stage_timings":[],"counters":[],"hot_functions":[],"top_bucket_label":"unknown","run_directory_path":"benchmarks/local-data/profiles/legacy"}]}"#;
    std::fs::write(&path, legacy).expect("write legacy record");

    let records = read_profile_runs(&path).expect("read");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].format_version, 1);
    assert_eq!(records[0].cases[0].case_id, "authored_case");
}

#[test]
fn roundtrip_preserves_null_commit() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-unknown");
    record.commit = None;
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    assert!(records[0].commit.is_none());
}

#[test]
fn roundtrip_preserves_null_sample_rate() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    assert!(record.sample_rate_hz.is_none());
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    assert!(records[0].sample_rate_hz.is_none());
}

#[test]
fn roundtrip_preserves_sample_rate() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record_b("2026-06-18T11-00-def5678");
    assert_eq!(record.sample_rate_hz, Some(1000.0));
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records[0].sample_rate_hz, Some(1000.0));
}

#[test]
fn case_with_empty_hot_functions_roundtrips() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].hot_functions = Vec::new();
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    assert!(records[0].cases[0].hot_functions.is_empty());
}

#[test]
fn case_with_multiple_args_roundtrips() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].args = vec!["foo.moth".to_string(), "--verbose".to_string()];
    append_profile_run(&path, &record).expect("append");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records[0].cases[0].args.len(), 2);
    assert_eq!(records[0].cases[0].args[0], "foo.moth");
    assert_eq!(records[0].cases[0].args[1], "--verbose");
}

#[test]
fn format_record_as_jsonl_produces_valid_json() {
    let record = test_record("2026-06-18T10-30-abc1234");
    let json = format_record_as_jsonl(&record);

    // Must start with { and end with }.
    assert!(json.starts_with('{'));
    assert!(json.ends_with('}'));
    // Must contain expected fields.
    assert!(json.contains(r#""format_version":2"#));
    assert!(json.contains(r#""case_id":"check_foo_bst""#));
    assert!(!json.contains(r#""case_name""#));
    assert!(json.contains(r#""run_id":"2026-06-18T10-30-abc1234""#));
    assert!(json.contains(r#""system_uuid":"TEST-UUID-001""#));
    assert!(json.contains(r#""filter_mode":"terse""#));
    assert!(json.contains(r#""cases":["#));
}
