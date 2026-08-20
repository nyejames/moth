use super::*;
use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{
    BenchmarkComparison, BenchmarkMeasurementIdentity, BenchmarkSystem, GitRevision, SuiteStats,
    calculate_group_stats,
};
use crate::test_fs::assert_path_missing;
use std::fs;
use std::sync::Mutex;
use tempfile::tempdir;

static THREAD_ENV_LOCK: Mutex<()> = Mutex::new(());

/// Panic-safe scoped environment variable guard.
///
/// WHAT: saves the current value of an env var, sets a new one (or removes it),
///   and restores the original on drop — even during unwinding.
/// WHY: direct `set_var`/`remove_var` without a guard leaves the environment
///   modified if an assertion panics, poisoning the test mutex and breaking
///   every subsequent test that depends on the same variable.
struct ScopedEnvVar {
    key: &'static str,
    saved: Option<std::ffi::OsString>,
    /// Once consumed by `restore`, the guard will not restore again on drop.
    restored: bool,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let saved = std::env::var_os(key);
        // SAFETY: no other thread accesses this env var while we hold THREAD_ENV_LOCK.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key,
            saved,
            restored: false,
        }
    }

    fn remove(key: &'static str) -> Self {
        let saved = std::env::var_os(key);
        // SAFETY: no other thread accesses this env var while we hold THREAD_ENV_LOCK.
        unsafe {
            std::env::remove_var(key);
        }
        Self {
            key,
            saved,
            restored: false,
        }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        // SAFETY: no other thread accesses this env var while we hold THREAD_ENV_LOCK.
        match &self.saved {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

fn cli_runner() -> BenchmarkRunner {
    BenchmarkRunner::Cli {
        command: CliBenchmarkCommand::Check,
        args: vec![
            "benchmarks/speed-test.moth".to_string(),
            "--terse".to_string(),
        ],
    }
}

fn benchmark_case() -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        case_id: "speed_test_check".to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: "speed_test".to_string(),
            source_fingerprint: "0123456789abcdef0123456789abcdef".to_string(),
            measurement_fingerprint: "fedcba9876543210fedcba9876543210".to_string(),
            timing_schema_version: 2,
        }),
        group_name: "core".to_string(),
        runner: cli_runner(),
        mean_ms: 40.0,
        median_ms: 39.0,
        stddev_ms: 3.0,
        observations: BenchmarkCaseObservations {
            timing_schema_version: 2,
            stage_timings: vec![BenchmarkMetric {
                name: "command.check.total".to_string(),
                value: 20.5,
            }],
            counters: vec![BenchmarkMetric {
                name: "source_file_count".to_string(),
                value: 8.0,
            }],
        },
    }
}

fn benchmark_run() -> BenchmarkRun {
    let cases = vec![benchmark_case()];

    BenchmarkRun {
        timestamp: BenchmarkTimestamp {
            year: 2026,
            month: 5,
            day: 10,
            hour: 15,
            minute: 21,
        },
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        git_revision: GitRevision {
            commit: Some("abc123".to_string()),
            dirty: Some(false),
        },
        system: BenchmarkSystem {
            system_uuid: "UUID123".to_string(),
            public_system_id: "B7F2A9".to_string(),
            display_name: "macOS M1".to_string(),
        },
        suite_kind: BenchmarkSuiteKind::EndToEndCli,
        groups: calculate_group_stats(&cases),
        suite: SuiteStats {
            average_ms: 40.0,
            case_spread_ms: 0.0,
        },
        cases,
        warmup_runs: 1,
        measured_iterations: 10,
        thread_count: None,
    }
}

fn current_record() -> LocalRunRecord {
    to_local_record(&benchmark_run())
}

fn read_one(line: &str) -> LocalRunRecord {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    fs::write(&path, line).expect("history fixture should be written");

    read_local_runs(&path)
        .expect("history should read")
        .into_iter()
        .next()
        .expect("one record should parse")
}

#[test]
fn v6_roundtrip_preserves_protocol_revision_runner_and_workload_identity() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let record = current_record();

    append_local_run(&path, &record).expect("current record should append");
    let records = read_local_runs(&path).expect("current record should read");

    assert_eq!(records, vec![record]);
    let parsed = &records[0];
    assert_eq!(
        parsed.benchmark_protocol_version,
        BENCHMARK_PROTOCOL_VERSION
    );
    assert_eq!(parsed.commit.as_deref(), Some("abc123"));
    assert_eq!(parsed.git_dirty, Some(false));
    assert_eq!(parsed.cases[0].case_id, "speed_test_check");
    assert_eq!(parsed.cases[0].workload_id.as_deref(), Some("speed_test"));
    assert_eq!(
        parsed.cases[0].source_fingerprint.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        parsed.cases[0].measurement_fingerprint.as_deref(),
        Some("fedcba9876543210fedcba9876543210")
    );
    assert_eq!(parsed.cases[0].runner, cli_runner());

    let json = fs::read_to_string(path).expect("record should be readable");
    assert!(json.contains(r#""format_version":8"#));
    assert!(json.contains(r#""benchmark_protocol_version":4"#));
    assert!(json.contains(r#""git_dirty":false"#));
    assert!(json.contains(r#""case_id":"speed_test_check""#));
    assert!(json.contains(r#""workload_id":"speed_test""#));
    assert!(json.contains(r#""kind":"cli""#));
    assert!(json.contains(r#""command":"check""#));
    assert!(json.contains(r#""args":["benchmarks/speed-test.moth","--terse"]"#));
}

#[test]
fn v6_roundtrip_preserves_frontend_profile_identity() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.suite_kind = BenchmarkSuiteKind::FrontendPhases
        .persisted_name()
        .to_string();
    record.primary_metric_name = BenchmarkSuiteKind::FrontendPhases
        .primary_metric_name()
        .to_string();
    record.cases[0].runner = BenchmarkRunner::Frontend {
        profile: FrontendBenchmarkProfile::Dev,
    };

    append_local_run(&path, &record).expect("frontend record should append");
    let parsed = read_local_runs(&path)
        .expect("frontend record should read")
        .pop()
        .expect("frontend record should exist");

    assert_eq!(parsed, record);
}

#[test]
fn v1_adapter_preserves_historical_name_and_assigns_protocol_zero() {
    let record = read_one(
        r#"{"format_version":1,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":"abc123","system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_mean_ms":68.0,"suite_stddev_ms":9.0,"cases":[{"name":"check_benchmarks_speed-test_bst","command":"check","args":["benchmarks/speed-test.bst"],"mean_ms":40.0,"stddev_ms":3.0}]}"#,
    );

    assert_eq!(record.format_version, 1);
    assert_eq!(record.benchmark_protocol_version, 0);
    assert_eq!(record.git_dirty, None);
    assert_eq!(record.cases[0].case_id, "check_benchmarks_speed-test_bst");
    assert_eq!(record.cases[0].workload_id, None);
    assert_eq!(record.cases[0].source_fingerprint, None);
    assert_eq!(record.cases[0].measurement_fingerprint, None);
    assert_eq!(record.cases[0].median_ms, 40.0);
    assert_eq!(record.groups[0].name, "core");
}

#[test]
fn v2_adapter_reads_grouped_statistics_without_observations() {
    let record = read_one(
        r#"{"format_version":2,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_average_ms":40.0,"suite_case_spread_ms":0.0,"groups":[{"name":"core","case_count":1,"average_ms":40.0}],"cases":[{"name":"check_speed-test_bst","group_name":"core","command":"check","args":["benchmarks/speed-test.bst"],"mean_ms":40.0,"median_ms":39.0,"stddev_ms":3.0}]}"#,
    );

    assert_eq!(record.format_version, 2);
    assert_eq!(record.benchmark_protocol_version, 0);
    assert!(record.cases[0].stage_timings.is_empty());
    assert!(record.cases[0].counters.is_empty());
}

#[test]
fn v3_adapter_reads_detailed_observations() {
    let record = read_one(
        r#"{"format_version":3,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":"abc123","system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_average_ms":68.0,"suite_case_spread_ms":9.0,"groups":[{"name":"core","case_count":1,"average_ms":40.0}],"cases":[{"name":"check_speed-test_bst","group_name":"core","command":"check","args":["benchmarks/speed-test.moth"],"mean_ms":40.0,"median_ms":39.0,"stddev_ms":3.0,"stage_timings":[{"name":"ast_ms","value":12.0}],"counters":[{"name":"token_count","value":100.0}]}]}"#,
    );

    assert_eq!(record.format_version, 3);
    assert_eq!(record.cases[0].stage_timings[0].name, "ast_ms");
    assert_eq!(record.cases[0].counters[0].value, 100.0);
}

#[test]
fn v4_adapter_reads_suite_identity_and_defaults_primary_metric() {
    let record = read_one(
        r#"{"format_version":4,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_kind":"frontend_phases","suite_average_ms":68.0,"suite_case_spread_ms":9.0,"groups":[],"cases":[]}"#,
    );

    assert_eq!(record.format_version, 4);
    assert_eq!(record.suite_kind, "frontend_phases");
    assert_eq!(record.primary_metric_name, "frontend_total_ms");
    assert_eq!(record.thread_count, None);
}

#[test]
fn v5_adapter_reads_fixed_thread_identity() {
    let record = read_one(
        r#"{"format_version":5,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_kind":"end_to_end_cli","primary_metric_name":"wall_time_ms","suite_average_ms":68.0,"suite_case_spread_ms":9.0,"thread_count":4,"groups":[],"cases":[]}"#,
    );

    assert_eq!(record.format_version, 5);
    assert_eq!(record.benchmark_protocol_version, 0);
    assert_eq!(record.thread_count, Some(4));
}

#[test]
fn dirty_record_is_ignored_by_latest_comparable_selection() {
    let mut dirty_newer = current_record();
    dirty_newer.system_uuid = "sys-a".to_string();
    dirty_newer.timestamp = "2026-05-11T15:21".to_string();
    dirty_newer.git_dirty = Some(true);

    let mut clean_older = current_record();
    clean_older.system_uuid = "sys-a".to_string();
    clean_older.timestamp = "2026-05-10T15:21".to_string();

    let records = [clean_older.clone(), dirty_newer.clone()];
    let latest = find_latest_matching_run(&records, "sys-a", BenchmarkSuiteKind::EndToEndCli, None)
        .expect("a clean record should remain selectable");
    assert_eq!(latest.timestamp, clean_older.timestamp);
    assert!(latest.git_dirty == Some(false));
}

#[test]
fn unknown_revision_record_is_ignored_by_latest_comparable_selection() {
    let mut unknown_newer = current_record();
    unknown_newer.system_uuid = "sys-a".to_string();
    unknown_newer.timestamp = "2026-05-11T15:21".to_string();
    unknown_newer.commit = None;

    let mut clean_older = current_record();
    clean_older.system_uuid = "sys-a".to_string();
    clean_older.timestamp = "2026-05-10T15:21".to_string();

    let records = [clean_older.clone(), unknown_newer.clone()];
    let latest = find_latest_matching_run(&records, "sys-a", BenchmarkSuiteKind::EndToEndCli, None)
        .expect("a clean record should remain selectable");
    assert_eq!(latest.timestamp, clean_older.timestamp);
    assert!(latest.commit.is_some());
}

#[test]
fn append_rejects_dirty_record() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.git_dirty = Some(true);

    let error =
        append_local_run(&path, &record).expect_err("a dirty record must not enter normal history");
    assert!(error.contains("clean and committed"));
    assert_path_missing(&path);
}

#[test]
fn append_rejects_unknown_revision_record() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.commit = None;

    let error = append_local_run(&path, &record)
        .expect_err("an unknown-revision record must not enter normal history");
    assert!(error.contains("clean and committed"));
    assert_path_missing(&path);
}

#[test]
fn old_records_never_match_current_protocol() {
    let old = read_one(
        r#"{"format_version":5,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_kind":"end_to_end_cli","primary_metric_name":"wall_time_ms","suite_average_ms":68.0,"suite_case_spread_ms":9.0,"thread_count":null,"groups":[],"cases":[]}"#,
    );
    let mut current = current_record();
    current.system_uuid = "sys-a".to_string();
    let records = vec![current.clone(), old];

    let latest = find_latest_matching_run(&records, "sys-a", BenchmarkSuiteKind::EndToEndCli, None)
        .expect("current protocol record should match");

    assert_eq!(latest.format_version, 8);
}

#[test]
fn matching_run_requires_exact_system_suite_thread_and_protocol() {
    let mut matching = current_record();
    matching.system_uuid = "sys-a".to_string();
    matching.thread_count = Some(4);

    assert!(
        find_latest_matching_run(
            std::slice::from_ref(&matching),
            "sys-a",
            BenchmarkSuiteKind::EndToEndCli,
            Some(4)
        )
        .is_some()
    );
    assert!(
        find_latest_matching_run(
            std::slice::from_ref(&matching),
            "sys-b",
            BenchmarkSuiteKind::EndToEndCli,
            Some(4)
        )
        .is_none()
    );
    assert!(
        find_latest_matching_run(
            std::slice::from_ref(&matching),
            "sys-a",
            BenchmarkSuiteKind::FrontendPhases,
            Some(4)
        )
        .is_none()
    );
    assert!(
        find_latest_matching_run(
            std::slice::from_ref(&matching),
            "sys-a",
            BenchmarkSuiteKind::EndToEndCli,
            None
        )
        .is_none()
    );
}

#[test]
fn future_and_malformed_versions_are_skipped_without_hiding_valid_records() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let valid = serde_json::to_string(&current_record()).expect("record should serialize");
    fs::write(
        &path,
        format!("{{\"format_version\":999}}\nnot-json\n{valid}\n"),
    )
    .expect("history fixture should be written");

    let records = read_local_runs(&path).expect("history should remain readable");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].format_version, 8);
}

#[test]
fn current_append_rejects_legacy_or_incomplete_records() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.benchmark_protocol_version = 0;

    let error = append_local_run(&path, &record).expect_err("protocol zero must not append");
    assert!(error.contains("legacy protocol"));
    assert_path_missing(&path);
}

#[test]
fn current_append_rejects_a_different_timing_schema() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.cases[0].timing_schema_version = Some(3);

    let error = append_local_run(&path, &record)
        .expect_err("new records must use the current timing schema");
    assert!(error.contains("incompatible current timing schema"));
    assert_path_missing(&path);
}

#[test]
fn current_history_rejects_obsolete_timing_metric_names() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.cases[0].stage_timings[0].name = "ast_ms".to_string();

    let error = append_local_run(&path, &record)
        .expect_err("current history must reject provisional timing names");
    assert!(error.contains("unknown timing schema metric 'ast_ms'"));
    assert_path_missing(&path);

    let invalid_line = serde_json::to_string(&record).expect("record should serialize");
    fs::write(&path, invalid_line).expect("invalid history fixture should be written");
    assert!(
        read_local_runs(&path)
            .expect("history should remain readable")
            .is_empty(),
        "current history with provisional timing names must be skipped"
    );
}

#[test]
fn current_history_rejects_empty_timing_evidence_on_append_and_read() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.cases[0].stage_timings.clear();

    let error = append_local_run(&path, &record)
        .expect_err("current history must reject empty timing evidence");
    assert!(error.contains("no metrics"), "unexpected error: {error}");
    assert_path_missing(&path);

    let invalid_line = serde_json::to_string(&record).expect("record should serialize");
    fs::write(&path, invalid_line).expect("invalid history fixture should be written");
    assert!(
        read_local_runs(&path)
            .expect("history should remain readable")
            .is_empty(),
        "current history with empty timing evidence must be skipped"
    );
}

#[test]
fn current_history_rejects_missing_command_total_on_append_and_read() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut record = current_record();
    record.cases[0].stage_timings = vec![LocalMetricRecord {
        name: "frontend.ast.total".to_string(),
        value: 12.0,
    }];

    let error = append_local_run(&path, &record)
        .expect_err("current history must require its command total");
    assert!(
        error.contains("command.check.total"),
        "unexpected error: {error}"
    );
    assert_path_missing(&path);

    let invalid_line = serde_json::to_string(&record).expect("record should serialize");
    fs::write(&path, invalid_line).expect("invalid history fixture should be written");
    assert!(
        read_local_runs(&path)
            .expect("history should remain readable")
            .is_empty(),
        "current history without its command total must be skipped"
    );
}

fn assert_non_finite_append_rejected(
    mut record: LocalRunRecord,
    mutate: impl FnOnce(&mut LocalRunRecord),
    field: &str,
) {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    mutate(&mut record);

    let error = append_local_run(&path, &record)
        .expect_err("non-finite current history values must not append");
    assert!(error.contains("finite"), "{field}: {error}");
    // Absence must be `NotFound`, not a metadata failure that merely looks absent.
    assert_path_missing(&path);
}

#[test]
fn current_append_rejects_non_finite_persisted_values() {
    assert_non_finite_append_rejected(
        current_record(),
        |record| record.suite_average_ms = f64::NAN,
        "suite average",
    );
    assert_non_finite_append_rejected(
        current_record(),
        |record| record.groups[0].average_ms = f64::NAN,
        "group average",
    );
    assert_non_finite_append_rejected(
        current_record(),
        |record| record.cases[0].mean_ms = f64::NAN,
        "case mean",
    );
    assert_non_finite_append_rejected(
        current_record(),
        |record| record.cases[0].stage_timings[0].value = f64::NAN,
        "stage timing",
    );
    assert_non_finite_append_rejected(
        current_record(),
        |record| record.cases[0].counters[0].value = f64::NAN,
        "counter",
    );
}

#[test]
fn persisted_schema_mismatch_reaches_non_comparable_comparison() {
    let directory = tempdir().expect("temporary directory should be created");
    let path = directory.path().join("runs.jsonl");
    let mut previous = current_record();
    previous.timestamp = "2026-05-09T15:21".to_string();
    previous.cases[0].timing_schema_version = Some(3);
    let current = current_record();
    let previous_json = serde_json::to_string(&previous).expect("previous record should serialize");
    let current_json = serde_json::to_string(&current).expect("current record should serialize");
    fs::write(&path, format!("{previous_json}\n{current_json}\n"))
        .expect("historical records should be written");

    let records = read_local_runs(&path).expect("historical records should remain readable");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].cases[0].timing_schema_version, Some(3));

    let previous_cases = to_case_results(&records[0]);
    let current_cases = to_case_results(&records[1]);
    let comparison = BenchmarkComparison::new(&current_cases, Some(&previous_cases));

    assert_eq!(
        comparison.timing_schema_changed_case_ids,
        ["speed_test_check"]
    );
    assert_eq!(comparison.compared_case_count, 0);
    assert!(comparison.overall_mean_delta_ms.is_none());
    assert_eq!(
        comparison.format_run_change_line(),
        "timing schema changed: 1 case (speed_test_check)"
    );
}

#[test]
fn to_case_results_preserves_current_identity_and_observations() {
    let cases = to_case_results(&current_record());

    assert_eq!(cases[0], benchmark_case());
}

#[test]
fn parse_thread_count_accepts_positive_and_rejects_invalid_values() {
    assert_eq!(parse_thread_count("4"), Ok(Some(4)));
    assert_eq!(parse_thread_count(" 8 "), Ok(Some(8)));
    assert!(parse_thread_count("").is_err());
    assert!(parse_thread_count("0").is_err());
    assert!(parse_thread_count("abc").is_err());
}

#[test]
fn effective_thread_count_distinguishes_unset_and_fixed() {
    let _guard = THREAD_ENV_LOCK
        .lock()
        .expect("environment lock should work");

    // The scoped guards restore the original env value on drop, including
    // during unwinding, so a panicking assertion cannot leave the environment
    // modified or poison the mutex for subsequent tests.
    {
        let _env = ScopedEnvVar::remove("RAYON_NUM_THREADS");
        assert_eq!(effective_thread_count(), Ok(None));
    }

    {
        let _env = ScopedEnvVar::set("RAYON_NUM_THREADS", "4");
        assert_eq!(effective_thread_count(), Ok(Some(4)));
    }
}

#[test]
fn thread_labels_distinguish_default_and_fixed() {
    assert_eq!(thread_identity_label(None), "default");
    assert_eq!(thread_identity_label(Some(4)), "fixed: 4");
    assert_eq!(thread_identity_suffix(None), "");
    assert_eq!(thread_identity_suffix(Some(4)), " [threads: fixed: 4]");
}
