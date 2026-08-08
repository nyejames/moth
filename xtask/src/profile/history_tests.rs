//! Tests for profile history storage and retrieval.

use super::*;
use crate::bench_types::{BenchmarkMeasurementIdentity, BenchmarkMetric, GitRevision};
use crate::profile::drift::{
    DriftCaseInput, DriftHotFunction, compute_drift, format_drift_markdown,
};
use std::collections::HashMap;

/// Build a test history record with one case.
fn test_record(run_id: &str) -> ProfileHistoryRecord {
    ProfileHistoryRecord {
        format_version: HISTORY_FORMAT_VERSION,
        profile_protocol_version: PROFILE_PROTOCOL_VERSION,
        run_id: run_id.to_string(),
        timestamp: "June 18th - 10:30".to_string(),
        git_revision: GitRevision {
            commit: Some("abc1234".to_string()),
            dirty: Some(false),
        },
        system_uuid: "TEST-UUID-001".to_string(),
        system_display: "Test System".to_string(),
        filter_mode: "terse".to_string(),
        sample_rate_hz: None,
        cases: vec![HistoryCaseRecord {
            case_id: "check_foo_bst".to_string(),
            identity: BenchmarkMeasurementIdentity {
                workload_id: "fixture".to_string(),
                source_fingerprint: "abc123".to_string(),
                measurement_fingerprint: "def456".to_string(),
                timing_schema_version: 1,
            },
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: vec!["foo.moth".to_string()],
            observation_wall_ms: 1234.5,
            sample_count: 500,
            sample_weight: 500.0,
            stage_timings: vec![
                BenchmarkMetric {
                    name: "command.check.total".to_string(),
                    value: 1234.5,
                },
                BenchmarkMetric {
                    name: "frontend.ast.total".to_string(),
                    value: 812.0,
                },
            ],
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
            run_directory_path: "2026-06-18T10-30-abc1234".to_string(),
        }],
    }
}

/// Build a second test record with different data.
fn test_record_b(run_id: &str) -> ProfileHistoryRecord {
    ProfileHistoryRecord {
        format_version: HISTORY_FORMAT_VERSION,
        profile_protocol_version: PROFILE_PROTOCOL_VERSION,
        run_id: run_id.to_string(),
        timestamp: "June 18th - 11:00".to_string(),
        git_revision: GitRevision {
            commit: Some("def5678".to_string()),
            dirty: Some(false),
        },
        system_uuid: "TEST-UUID-001".to_string(),
        system_display: "Test System".to_string(),
        filter_mode: "terse".to_string(),
        sample_rate_hz: Some(1000.0),
        cases: vec![HistoryCaseRecord {
            case_id: "check_foo_bst".to_string(),
            identity: BenchmarkMeasurementIdentity {
                workload_id: "fixture".to_string(),
                source_fingerprint: "def567".to_string(),
                measurement_fingerprint: "ghi789".to_string(),
                timing_schema_version: 1,
            },
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: vec!["foo.moth".to_string()],
            observation_wall_ms: 1400.0,
            sample_count: 600,
            sample_weight: 600.0,
            stage_timings: vec![
                BenchmarkMetric {
                    name: "command.check.total".to_string(),
                    value: 1400.0,
                },
                BenchmarkMetric {
                    name: "frontend.ast.total".to_string(),
                    value: 900.0,
                },
            ],
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
            run_directory_path: "2026-06-18T11-00-def5678".to_string(),
        }],
    }
}

/// Unwrap every stored record as the current variant.
fn current_records(records: Vec<StoredProfileHistoryRecord>) -> Vec<ProfileHistoryRecord> {
    records
        .into_iter()
        .map(|stored| match stored {
            StoredProfileHistoryRecord::Current(record) => record,
            StoredProfileHistoryRecord::Legacy(_) => {
                panic!("expected a current record, found legacy")
            }
        })
        .collect()
}

fn drift_input(case: &HistoryCaseRecord) -> DriftCaseInput {
    DriftCaseInput {
        case_id: case.case_id.clone(),
        identity: case.identity.clone(),
        command: case.command.clone(),
        args: case.args.clone(),
        stage_timings: case.stage_timings.clone(),
        counters: case.counters.clone(),
        hot_functions: case
            .hot_functions
            .iter()
            .map(|function| DriftHotFunction {
                name: function.name.clone(),
                bucket_label: function.bucket_label.clone(),
                inclusive_samples: function.inclusive_samples,
                inclusive_pct: function.inclusive_pct,
            })
            .collect(),
    }
}

#[test]
fn append_and_read_single_record() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
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

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
    assert_eq!(records[1].run_id, "2026-06-18T11-00-def5678");
    assert_eq!(records[1].sample_rate_hz, Some(1000.0));
}

#[test]
fn current_profile_history_rejects_obsolete_timing_metric_names() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].stage_timings[1].name = "ast_ms".to_string();

    let error = append_profile_run(&path, &record)
        .expect_err("current profile history must reject provisional timing names");
    assert!(error.contains("unknown timing schema metric 'ast_ms'"));

    let invalid_line = serde_json::to_string(&record).expect("record should serialize");
    std::fs::write(&path, invalid_line).expect("invalid profile history fixture should be written");
    assert!(
        read_profile_runs(&path)
            .expect("profile history should remain readable")
            .is_empty(),
        "current profile history with provisional timing names must be skipped"
    );
}

#[test]
fn current_profile_history_rejects_empty_timing_evidence_on_append_and_read() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let mut record = test_record("2026-06-18T10-30-empty");
    record.cases[0].stage_timings.clear();

    let error = append_profile_run(&path, &record)
        .expect_err("current profile history must reject empty timing evidence");
    assert!(error.contains("no metrics"), "unexpected error: {error}");
    assert!(!path.exists());

    let invalid_line = serde_json::to_string(&record).expect("record should serialize");
    std::fs::write(&path, invalid_line).expect("invalid profile history fixture should be written");
    assert!(
        read_profile_runs(&path)
            .expect("profile history should remain readable")
            .is_empty(),
        "current profile history with empty timing evidence must be skipped"
    );
}

#[test]
fn current_profile_history_rejects_missing_command_total_on_append_and_read() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let mut record = test_record("2026-06-18T10-30-missing-total");
    record.cases[0].stage_timings = vec![BenchmarkMetric {
        name: "frontend.ast.total".to_string(),
        value: 812.0,
    }];

    let error = append_profile_run(&path, &record)
        .expect_err("current profile history must require its command total");
    assert!(
        error.contains("command.check.total"),
        "unexpected error: {error}"
    );
    assert!(!path.exists());

    let invalid_line = serde_json::to_string(&record).expect("record should serialize");
    std::fs::write(&path, invalid_line).expect("invalid profile history fixture should be written");
    assert!(
        read_profile_runs(&path)
            .expect("profile history should remain readable")
            .is_empty(),
        "current profile history without its command total must be skipped"
    );
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
    let valid_line = serde_json::to_string(&record).expect("record should serialize");
    let content = format!("this is not json\n{}\n", valid_line);
    std::fs::write(&path, content).expect("write");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
}

#[test]
fn unknown_format_version_is_skipped() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    let valid_line = serde_json::to_string(&record).expect("record should serialize");
    // Write a line with a future format_version.
    let content = r#"{"format_version":999,"run_id":"future","timestamp":"now","system_uuid":"x","system_display":"x","filter_mode":"terse","cases":[]}"#;
    std::fs::write(&path, format!("{}\n{}\n", content, valid_line)).expect("write");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, "2026-06-18T10-30-abc1234");
}

#[test]
fn roundtrip_preserves_case_data() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
    let case = &records[0].cases[0];

    assert_eq!(case.observation_wall_ms, 1234.5);
    assert_eq!(case.sample_count, 500);
    assert_eq!(case.sample_weight, 500.0);
    assert_eq!(case.stage_timings.len(), 2);
    assert_eq!(case.stage_timings[1].name, "frontend.ast.total");
    assert_eq!(case.stage_timings[1].value, 812.0);
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
fn legacy_v1_case_name_is_adapted_to_legacy_shape() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let legacy = r#"{"format_version":1,"run_id":"legacy","timestamp":"June 18th - 09:00","commit":null,"system_uuid":"TEST-UUID-001","system_display":"Test System","filter_mode":"terse","sample_rate_hz":null,"cases":[{"case_name":"authored_case","group_name":"core","command":"check","args":["foo.moth"],"observation_wall_ms":10.0,"sample_count":2,"sample_weight":2.0,"stage_timings":[],"counters":[],"hot_functions":[],"top_bucket_label":"unknown","run_directory_path":"benchmarks/local-data/profiles/legacy"}]}"#;
    std::fs::write(&path, legacy).expect("write legacy record");

    let records = read_profile_runs(&path).expect("read");
    assert_eq!(records.len(), 1);

    match &records[0] {
        StoredProfileHistoryRecord::Legacy(record) => {
            assert_eq!(record.format_version, 1);
            assert_eq!(record.cases[0].case_id, "authored_case");
            assert!(record.cases[0].identity.is_none());
        }
        StoredProfileHistoryRecord::Current(_) => {
            panic!("v1 line must adapt to the legacy shape");
        }
    }
}

#[test]
fn current_record_without_commit_fails_its_line() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let line = r#"{"format_version":4,"profile_protocol_version":2,"run_id":"x","timestamp":"t","commit":null,"git_dirty":false,"system_uuid":"s","system_display":"d","filter_mode":"terse","sample_rate_hz":null,"cases":[]}"#;
    std::fs::write(&path, line).expect("write");

    let records = read_profile_runs(&path).expect("read");
    assert!(
        records.is_empty(),
        "a current record without a commit is malformed"
    );
}

#[test]
fn current_record_with_null_identity_fails_its_line() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");
    let line = r#"{"format_version":4,"profile_protocol_version":2,"run_id":"x","timestamp":"t","commit":"abc","git_dirty":false,"system_uuid":"s","system_display":"d","filter_mode":"terse","sample_rate_hz":null,"cases":[{"case_id":"c","identity":null,"group_name":"core","command":"check","args":[],"observation_wall_ms":1.0,"sample_count":1,"sample_weight":1.0,"stage_timings":[],"counters":[],"hot_functions":[],"top_bucket_label":"unknown","run_directory_path":"p"}]}"#;
    std::fs::write(&path, line).expect("write");

    let records = read_profile_runs(&path).expect("read");
    assert!(
        records.is_empty(),
        "a current record with a null case identity is malformed"
    );
}

#[test]
fn append_rejects_dirty_record() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.git_revision.dirty = Some(true);

    let error = append_profile_run(&path, &record)
        .expect_err("a dirty record must not enter current profile history");
    assert!(error.contains("clean and committed"));
    assert!(!path.exists());
}

#[test]
fn append_rejects_unknown_revision_record() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-unknown");
    record.git_revision.commit = None;

    let error = append_profile_run(&path, &record)
        .expect_err("an unknown-revision record must not enter current profile history");
    assert!(error.contains("clean and committed"));
    assert!(!path.exists());
}

#[test]
fn append_rejects_different_timing_schema() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-schema");
    record.cases[0].identity.timing_schema_version = 2;

    let error = append_profile_run(&path, &record)
        .expect_err("new profile records must use the current timing schema");
    assert!(error.contains("incompatible current timing schema"));
    assert!(!path.exists());
}

#[test]
fn append_rejects_incompatible_format_version() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-format");
    record.format_version = HISTORY_FORMAT_VERSION - 1;

    let error = append_profile_run(&path, &record)
        .expect_err("new profile records must use the current history format");
    assert!(error.contains("format version"));
    assert!(!path.exists());
}

#[test]
fn append_rejects_incompatible_protocol_version() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-protocol");
    record.profile_protocol_version = PROFILE_PROTOCOL_VERSION - 1;

    let error = append_profile_run(&path, &record)
        .expect_err("new profile records must use the current profile protocol");
    assert!(error.contains("protocol version"));
    assert!(!path.exists());
}

#[test]
fn persisted_schema_mismatch_reaches_profile_drift_without_numeric_comparison() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut previous = test_record("2026-06-18T10-30-previous");
    previous.cases[0].identity.timing_schema_version = 2;
    previous.cases[0].stage_timings[1].value = 1.0;
    previous.cases[0].counters[0].value = 1.0;
    previous.cases[0].hot_functions[0].inclusive_pct = 1.0;

    let mut current = test_record("2026-06-18T11-00-current");
    current.cases[0].stage_timings[1].value = 999.0;
    current.cases[0].counters[0].value = 999999.0;
    current.cases[0].hot_functions[0].inclusive_pct = 90.0;

    let previous_json = serde_json::to_string(&previous).expect("previous record should serialize");
    let current_json = serde_json::to_string(&current).expect("current record should serialize");
    std::fs::write(&path, format!("{previous_json}\n{current_json}\n"))
        .expect("profile history fixture should be written");

    let records = current_records(read_profile_runs(&path).expect("profile history should read"));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].cases[0].identity.timing_schema_version, 2);

    let report = compute_drift(
        &[drift_input(&records[1].cases[0])],
        &records[0],
        &HashMap::from([(String::from("check_foo_bst"), 999.0)]),
    );

    assert_eq!(report.timing_schema_changed_case_ids, ["check_foo_bst"]);
    assert!(report.measurement_changed_case_ids.is_empty());
    assert!(report.function_increases.is_empty());
    assert!(report.function_decreases.is_empty());
    assert!(report.stage_movements.is_empty());
    assert!(report.counter_movements.is_empty());
    assert_eq!(
        format_drift_markdown(&report)
            .matches("timing schema changed")
            .count(),
        1
    );
}

#[test]
fn unicode_and_escaping_roundtrip_through_serde() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].case_id = "case_\"quoted\"_ünïcode".to_string();
    record.cases[0].args = vec!["path with \"quotes\"".to_string(), "ünïcode".to_string()];
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert_eq!(records[0].cases[0].case_id, "case_\"quoted\"_ünïcode");
    assert_eq!(
        records[0].cases[0].args,
        vec!["path with \"quotes\"".to_string(), "ünïcode".to_string()]
    );
}

#[test]
fn non_finite_data_fails_serialization() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].observation_wall_ms = f64::NAN;

    let error = append_profile_run(&path, &record)
        .expect_err("non-finite observations must be rejected before writing");
    assert!(error.contains("finite"));
    assert!(!path.exists());
}

#[test]
fn non_finite_sample_rate_fails_serialization() {
    for (index, sample_rate_hz) in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY]
        .into_iter()
        .enumerate()
    {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let path = temp_dir.path().join(format!("profile-runs-{index}.jsonl"));
        let mut record = test_record("2026-06-18T10-30-invalid-rate");
        record.sample_rate_hz = Some(sample_rate_hz);

        let error = append_profile_run(&path, &record)
            .expect_err("non-finite sample rates must be rejected before writing");
        assert!(error.contains("finite"));
        assert!(!path.exists());
    }
}

#[test]
fn roundtrip_preserves_null_sample_rate() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record("2026-06-18T10-30-abc1234");
    assert!(record.sample_rate_hz.is_none());
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert!(records[0].sample_rate_hz.is_none());
}

#[test]
fn roundtrip_preserves_sample_rate() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let record = test_record_b("2026-06-18T11-00-def5678");
    assert_eq!(record.sample_rate_hz, Some(1000.0));
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert_eq!(records[0].sample_rate_hz, Some(1000.0));
}

#[test]
fn case_with_empty_hot_functions_roundtrips() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].hot_functions = Vec::new();
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert!(records[0].cases[0].hot_functions.is_empty());
}

#[test]
fn case_with_multiple_args_roundtrips() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let path = temp_dir.path().join("profile-runs.jsonl");

    let mut record = test_record("2026-06-18T10-30-abc1234");
    record.cases[0].args = vec!["foo.moth".to_string(), "--verbose".to_string()];
    append_profile_run(&path, &record).expect("append");

    let records = current_records(read_profile_runs(&path).expect("read"));
    assert_eq!(records[0].cases[0].args.len(), 2);
    assert_eq!(records[0].cases[0].args[0], "foo.moth");
    assert_eq!(records[0].cases[0].args[1], "--verbose");
}

#[test]
fn current_serialization_is_flat_and_versioned() {
    let record = test_record("2026-06-18T10-30-abc1234");
    let json = serde_json::to_string(&record).expect("record should serialize");

    assert!(json.starts_with('{'));
    assert!(json.ends_with('}'));
    assert!(json.contains(r#""format_version":4"#));
    assert!(json.contains(r#""profile_protocol_version":2"#));
    assert!(json.contains(r#""commit":"abc1234""#));
    assert!(json.contains(r#""git_dirty":false"#));
    assert!(json.contains(r#""case_id":"check_foo_bst""#));
    assert!(!json.contains(r#""case_name""#));
    assert!(json.contains(r#""run_id":"2026-06-18T10-30-abc1234""#));
    assert!(json.contains(r#""system_uuid":"TEST-UUID-001""#));
    assert!(json.contains(r#""filter_mode":"terse""#));
    assert!(json.contains(r#""cases":["#));
}
