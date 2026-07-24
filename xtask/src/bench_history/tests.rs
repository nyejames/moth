use super::*;
use std::fs;
use std::sync::Mutex;

/// Serializes tests that mutate the `RAYON_NUM_THREADS` environment variable.
static THREAD_ENV_LOCK: Mutex<()> = Mutex::new(());

fn make_record(system_uuid: &str, timestamp: &str) -> LocalRunRecord {
    LocalRunRecord {
        format_version: 5,
        timestamp: timestamp.to_string(),
        month_key: "2026-05".to_string(),
        commit: Some("abc123".to_string()),
        system_uuid: system_uuid.to_string(),
        public_system_id: "B7F2A9".to_string(),
        display_name: "macOS M1".to_string(),
        warmup_runs: 1,
        measured_iterations: 10,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: "wall_time_ms".to_string(),
        suite_average_ms: 68.0,
        suite_case_spread_ms: 9.0,
        thread_count: None,
        groups: vec![LocalGroupRecord {
            name: "core".to_string(),
            case_count: 1,
            average_ms: 40.0,
        }],
        cases: vec![LocalCaseRecord {
            name: "check_speed-test_bst".to_string(),
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: vec!["benchmarks/speed-test.moth".to_string()],
            mean_ms: 40.0,
            median_ms: 39.0,
            stddev_ms: 3.0,
            stage_timings: Vec::new(),
            counters: Vec::new(),
        }],
    }
}

#[test]
fn test_json_escape() {
    assert_eq!(json_escape("hello"), "hello");
    assert_eq!(json_escape("with \"quotes\""), "with \\\"quotes\\\"");
    assert_eq!(json_escape("a\\b"), "a\\\\b");
    assert_eq!(json_escape("line1\nline2"), "line1\\nline2");
    assert_eq!(json_escape("tab\there"), "tab\\there");
}

#[test]
fn test_read_empty_file() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_empty.jsonl");
    let _ = fs::remove_file(&path);

    let runs = read_local_runs(&path).unwrap();
    assert!(runs.is_empty());
}

#[test]
fn test_roundtrip_jsonl() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_roundtrip.jsonl");
    let _ = fs::remove_file(&path);

    let record = make_record("UUID123", "2026-05-10T15:21");
    append_local_run(&path, &record).unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0], record);

    // Cleanup
    let _ = fs::remove_file(&path);
}

#[test]
fn test_v1_record_parses_into_v2_in_memory_shape() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v1_parse.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(
        &path,
        r#"{"format_version":1,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":"abc123","system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_mean_ms":68.0,"suite_stddev_ms":9.0,"cases":[{"name":"check_benchmarks_speed-test_bst","command":"check","args":["benchmarks/speed-test.moth"],"mean_ms":40.0,"stddev_ms":3.0}]}"#,
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].format_version, 1);
    assert_eq!(runs[0].suite_average_ms, 68.0);
    assert_eq!(runs[0].suite_case_spread_ms, 9.0);
    assert_eq!(runs[0].cases[0].group_name, "core");
    assert_eq!(runs[0].cases[0].median_ms, 40.0);
    assert_eq!(runs[0].groups[0].name, "core");

    let _ = fs::remove_file(&path);
}

#[test]
fn test_v1_group_inference_for_docs_and_stress() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v1_groups.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(
        &path,
        r#"{"format_version":1,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_mean_ms":20.0,"suite_stddev_ms":5.0,"cases":[{"name":"check_docs","command":"check","args":["docs"],"mean_ms":30.0,"stddev_ms":1.0},{"name":"check_benchmarks_template-stress_bst","command":"check","args":["benchmarks/template-stress.moth"],"mean_ms":10.0,"stddev_ms":1.0}]}"#,
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();

    assert_eq!(runs[0].cases[0].group_name, "docs");
    assert_eq!(runs[0].cases[1].group_name, "stress");
    assert_eq!(runs[0].groups.len(), 2);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_find_latest_matching_run() {
    let runs = vec![
        make_record("sys-a", "2026-05-10T10:00"),
        make_record("sys-b", "2026-05-10T11:00"),
        make_record("sys-a", "2026-05-10T12:00"),
    ];

    let latest = find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, None);
    assert!(latest.is_some());
    assert_eq!(latest.unwrap().timestamp, "2026-05-10T12:00");

    let latest_b = find_latest_matching_run(&runs, "sys-b", BenchmarkSuiteKind::EndToEndCli, None);
    assert_eq!(latest_b.unwrap().timestamp, "2026-05-10T11:00");

    let latest_c = find_latest_matching_run(&runs, "sys-c", BenchmarkSuiteKind::EndToEndCli, None);
    assert!(latest_c.is_none());
}

#[test]
fn test_find_latest_skips_unknown_version() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_version.jsonl");
    let _ = fs::remove_file(&path);

    let record = make_record("sys-a", "2026-05-10T10:00");
    append_local_run(&path, &record).unwrap();

    // Manually append a record with version 999
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(
        file,
        r#"{{"format_version":999,"timestamp":"2026-05-10T11:00","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"XXXX","display_name":"Unknown","warmup_runs":1,"measured_iterations":10,"suite_kind":"end_to_end_cli","primary_metric_name":"wall_time_ms","suite_average_ms":100.0,"suite_case_spread_ms":0.0,"groups":[],"cases":[]}}"#
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].format_version, 5);

    // Cleanup
    let _ = fs::remove_file(&path);
}

#[test]
fn test_v3_record_defaults_to_end_to_end_cli() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v3_defaults.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(
        &path,
        r#"{"format_version":3,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":"abc123","system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_average_ms":68.0,"suite_case_spread_ms":9.0,"groups":[{"name":"core","case_count":1,"average_ms":40.0}],"cases":[{"name":"check_speed-test_bst","group_name":"core","command":"check","args":["benchmarks/speed-test.moth"],"mean_ms":40.0,"median_ms":39.0,"stddev_ms":3.0,"stage_timings":[],"counters":[]}]}"#,
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].suite_kind, "end_to_end_cli");
    assert_eq!(runs[0].primary_metric_name, "wall_time_ms");

    let _ = fs::remove_file(&path);
}

#[test]
fn test_v4_record_roundtrip_includes_suite_kind() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v4_roundtrip.jsonl");
    let _ = fs::remove_file(&path);

    let mut record = make_record("sys-a", "2026-05-10T15:21");
    record.suite_kind = "frontend_phases".to_string();
    record.primary_metric_name = "frontend_total_ms".to_string();
    append_local_run(&path, &record).unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].suite_kind, "frontend_phases");
    assert_eq!(runs[0].primary_metric_name, "frontend_total_ms");

    let _ = fs::remove_file(&path);
}

#[test]
fn test_v4_missing_primary_metric_defaults_from_suite_kind() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v4_primary_default.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(
        &path,
        r#"{"format_version":4,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_kind":"frontend_phases","suite_average_ms":68.0,"suite_case_spread_ms":9.0,"groups":[],"cases":[]}"#,
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].suite_kind, "frontend_phases");
    assert_eq!(runs[0].primary_metric_name, "frontend_total_ms");

    let _ = fs::remove_file(&path);
}

#[test]
fn test_find_latest_matching_run_filters_by_suite_kind() {
    let mut cli_record = make_record("sys-a", "2026-05-10T10:00");
    cli_record.suite_kind = "end_to_end_cli".to_string();

    let mut frontend_record = make_record("sys-a", "2026-05-10T11:00");
    frontend_record.suite_kind = "frontend_phases".to_string();

    let runs = vec![cli_record, frontend_record];

    let latest_cli =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, None);
    assert!(latest_cli.is_some());
    assert_eq!(latest_cli.unwrap().timestamp, "2026-05-10T10:00");

    let latest_frontend =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::FrontendPhases, None);
    assert!(latest_frontend.is_some());
    assert_eq!(latest_frontend.unwrap().timestamp, "2026-05-10T11:00");

    let latest_other_system =
        find_latest_matching_run(&runs, "sys-b", BenchmarkSuiteKind::FrontendPhases, None);
    assert!(latest_other_system.is_none());
}

#[test]
fn test_v4_record_without_thread_count_parses_as_default() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v4_no_thread_count.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(
        &path,
        r#"{"format_version":4,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_kind":"end_to_end_cli","suite_average_ms":68.0,"suite_case_spread_ms":9.0,"groups":[],"cases":[]}"#,
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].thread_count, None);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_v5_fixed_thread_record_roundtrips() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v5_thread_roundtrip.jsonl");
    let _ = fs::remove_file(&path);

    let mut record = make_record("sys-a", "2026-05-10T15:21");
    record.thread_count = Some(4);
    append_local_run(&path, &record).unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].thread_count, Some(4));

    let _ = fs::remove_file(&path);
}

#[test]
fn test_v5_null_thread_count_parses_as_default() {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_v5_null_thread.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(
        &path,
        r#"{"format_version":5,"timestamp":"2026-05-10T15:21","month_key":"2026-05","commit":null,"system_uuid":"sys-a","public_system_id":"B7F2A9","display_name":"macOS M1","warmup_runs":1,"measured_iterations":10,"suite_kind":"end_to_end_cli","suite_average_ms":68.0,"suite_case_spread_ms":9.0,"thread_count":null,"groups":[],"cases":[]}"#,
    )
    .unwrap();

    let runs = read_local_runs(&path).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].thread_count, None);

    let _ = fs::remove_file(&path);
}

#[test]
fn test_find_latest_default_never_matches_fixed() {
    let mut default_run = make_record("sys-a", "2026-05-10T10:00");
    default_run.thread_count = None;

    let mut fixed_run = make_record("sys-a", "2026-05-10T11:00");
    fixed_run.thread_count = Some(4);

    let runs = vec![default_run, fixed_run];

    // A default-thread query must not match the fixed-thread run.
    let latest_default =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, None);
    assert_eq!(latest_default.unwrap().timestamp, "2026-05-10T10:00");

    // A fixed-thread query must not match the default-thread run.
    let latest_fixed =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, Some(4));
    assert_eq!(latest_fixed.unwrap().timestamp, "2026-05-10T11:00");

    // A non-existent fixed count matches neither.
    let latest_other =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, Some(2));
    assert!(latest_other.is_none());
}

#[test]
fn test_find_latest_fixed_counts_never_match_each_other() {
    let mut one_thread = make_record("sys-a", "2026-05-10T10:00");
    one_thread.thread_count = Some(1);

    let mut two_thread = make_record("sys-a", "2026-05-10T11:00");
    two_thread.thread_count = Some(2);

    let runs = vec![one_thread, two_thread];

    let latest_one =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, Some(1));
    assert_eq!(latest_one.unwrap().timestamp, "2026-05-10T10:00");

    let latest_two =
        find_latest_matching_run(&runs, "sys-a", BenchmarkSuiteKind::EndToEndCli, Some(2));
    assert_eq!(latest_two.unwrap().timestamp, "2026-05-10T11:00");

    // Each count matches only its own run, not the other.
    assert_ne!(
        latest_one.unwrap().thread_count,
        latest_two.unwrap().thread_count
    );
}

#[test]
fn test_parse_thread_count_positive_returns_fixed() {
    // A valid positive integer is accepted as a fixed thread count.
    // Whitespace is trimmed so " 8 " is equivalent to "8".
    assert_eq!(parse_thread_count("4"), Ok(Some(4)));
    assert_eq!(parse_thread_count(" 8 "), Ok(Some(8)));
    assert_eq!(parse_thread_count("1"), Ok(Some(1)));
}

#[test]
fn test_parse_thread_count_empty_returns_error() {
    let result = parse_thread_count("");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("empty"),
        "error should mention empty: got {err}"
    );
}

#[test]
fn test_parse_thread_count_zero_returns_error() {
    let result = parse_thread_count("0");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("0"), "error should mention zero: got {err}");
}

#[test]
fn test_parse_thread_count_malformed_returns_error() {
    let result = parse_thread_count("abc");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("abc"),
        "error should mention the bad value: got {err}"
    );
}

#[test]
fn test_thread_identity_label_default() {
    assert_eq!(thread_identity_label(None), "default");
}

#[test]
fn test_thread_identity_label_fixed() {
    assert_eq!(thread_identity_label(Some(1)), "fixed: 1");
    assert_eq!(thread_identity_label(Some(4)), "fixed: 4");
}

#[test]
fn test_effective_thread_count_unset_returns_default() {
    let _guard = THREAD_ENV_LOCK.lock().unwrap();
    // Save and remove the variable so the unset path is exercised regardless
    // of the ambient environment. This test is the sole owner of the env var
    // mutation, so it is safe within the test process.
    let saved = std::env::var("RAYON_NUM_THREADS").ok();
    unsafe {
        std::env::remove_var("RAYON_NUM_THREADS");
    }

    assert_eq!(effective_thread_count(), Ok(None));

    if let Some(value) = saved {
        unsafe {
            std::env::set_var("RAYON_NUM_THREADS", value);
        }
    }
}

#[test]
fn test_effective_thread_count_empty_string_returns_error() {
    let _guard = THREAD_ENV_LOCK.lock().unwrap();
    let saved = std::env::var("RAYON_NUM_THREADS").ok();
    unsafe {
        std::env::set_var("RAYON_NUM_THREADS", "");
    }

    let result = effective_thread_count();
    assert!(
        result.is_err(),
        "empty string should be an error: got {result:?}"
    );

    if let Some(value) = saved {
        unsafe {
            std::env::set_var("RAYON_NUM_THREADS", value);
        }
    } else {
        unsafe {
            std::env::remove_var("RAYON_NUM_THREADS");
        }
    }
}

#[test]
fn test_effective_thread_count_valid_positive_returns_fixed() {
    let _guard = THREAD_ENV_LOCK.lock().unwrap();
    let saved = std::env::var("RAYON_NUM_THREADS").ok();
    unsafe {
        std::env::set_var("RAYON_NUM_THREADS", "4");
    }

    assert_eq!(effective_thread_count(), Ok(Some(4)));

    if let Some(value) = saved {
        unsafe {
            std::env::set_var("RAYON_NUM_THREADS", value);
        }
    } else {
        unsafe {
            std::env::remove_var("RAYON_NUM_THREADS");
        }
    }
}

#[test]
fn test_get_commit_hash_does_not_panic() {
    // The function should never panic, regardless of git availability.
    // It returns Option<String>, so the caller can handle None gracefully.
    let _ = get_commit_hash();
}

#[test]
fn test_to_case_results() {
    let record = make_record("sys-a", "2026-05-10T15:21");
    let cases = to_case_results(&record);

    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_name, "check_speed-test_bst");
    assert_eq!(cases[0].group_name, "core");
    assert_eq!(cases[0].mean_ms, 40.0);
    assert_eq!(cases[0].median_ms, 39.0);
}

#[test]
fn test_to_group_stats() {
    let record = make_record("sys-a", "2026-05-10T15:21");
    let groups = to_group_stats(&record);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].group_name, "core");
    assert_eq!(groups[0].case_count, 1);
    assert_eq!(groups[0].average_ms, 40.0);
}

#[test]
fn test_format_record_as_jsonl_structure() {
    let record = make_record("sys-a", "2026-05-10T15:21");
    let json = format_record_as_jsonl(&record);

    // Should be a single-line JSON object
    assert!(json.starts_with('{'));
    assert!(json.ends_with('}'));

    // Should contain expected fields
    assert!(json.contains(r#""format_version":5"#));
    assert!(json.contains(r#""timestamp":"2026-05-10T15:21""#));
    assert!(json.contains(r#""commit":"abc123""#));
    assert!(json.contains(r#""system_uuid":"sys-a""#));
    assert!(json.contains(r#""suite_average_ms":68"#));
    assert!(json.contains(r#""suite_case_spread_ms":9"#));
    assert!(json.contains(r#""groups":"#));
    assert!(json.contains(r#""group_name":"core""#));
    assert!(json.contains(r#""median_ms":39"#));
    assert!(json.contains(r#""cases":"#));
}

#[test]
fn test_format_record_with_null_commit() {
    let mut record = make_record("sys-a", "2026-05-10T15:21");
    record.commit = None;
    let json = format_record_as_jsonl(&record);
    assert!(json.contains(r#""commit":null"#));
}

#[test]
fn test_to_local_record_preserves_options() {
    use crate::bench_time::BenchmarkTimestamp;
    use crate::bench_types::{
        BenchmarkCaseResult, BenchmarkRun, BenchmarkSuiteKind, BenchmarkSystem, SuiteStats,
        calculate_group_stats,
    };

    let cases = vec![BenchmarkCaseResult {
        case_name: "check_speed-test_bst".to_string(),
        group_name: "core".to_string(),
        command: "check".to_string(),
        args: vec!["benchmarks/speed-test.moth".to_string()],
        mean_ms: 40.0,
        median_ms: 40.0,
        stddev_ms: 3.0,
        observations: Default::default(),
    }];

    let run = BenchmarkRun {
        timestamp: BenchmarkTimestamp {
            year: 2026,
            month: 5,
            day: 10,
            hour: 15,
            minute: 21,
        },
        commit: Some("abc123".to_string()),
        system: BenchmarkSystem {
            system_uuid: "UUID123".to_string(),
            public_system_id: "B7F2A9".to_string(),
            display_name: "macOS M1".to_string(),
        },
        suite_kind: BenchmarkSuiteKind::EndToEndCli,
        groups: calculate_group_stats(&cases),
        cases,
        suite: SuiteStats {
            average_ms: 68.0,
            case_spread_ms: 9.0,
        },
        warmup_runs: 3,
        measured_iterations: 15,
        thread_count: None,
    };

    let record = to_local_record(&run, Some("abc123".to_string()));
    assert_eq!(record.warmup_runs, 3);
    assert_eq!(record.measured_iterations, 15);
    assert_eq!(record.groups.len(), 1);
    assert_eq!(record.groups[0].name, "core");
    assert_eq!(record.cases[0].group_name, "core");
    assert_eq!(record.cases[0].median_ms, 40.0);
}

#[test]
fn test_to_local_record_preserves_detailed_observations() {
    use crate::bench_time::BenchmarkTimestamp;
    use crate::bench_types::{
        BenchmarkCaseObservations, BenchmarkCaseResult, BenchmarkMetric, BenchmarkRun,
        BenchmarkSuiteKind, BenchmarkSystem, SuiteStats, calculate_group_stats,
    };

    let cases = vec![BenchmarkCaseResult {
        case_name: "check_docs".to_string(),
        group_name: "docs".to_string(),
        command: "check".to_string(),
        args: vec!["docs".to_string()],
        mean_ms: 100.0,
        median_ms: 99.0,
        stddev_ms: 1.0,
        observations: BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 20.5,
            }],
            counters: vec![BenchmarkMetric {
                name: "StringTable/full clone count".to_string(),
                value: 8.0,
            }],
        },
    }];

    let run = BenchmarkRun {
        timestamp: BenchmarkTimestamp {
            year: 2026,
            month: 5,
            day: 10,
            hour: 15,
            minute: 21,
        },
        commit: None,
        system: BenchmarkSystem {
            system_uuid: "UUID123".to_string(),
            public_system_id: "B7F2A9".to_string(),
            display_name: "macOS M1".to_string(),
        },
        suite_kind: BenchmarkSuiteKind::EndToEndCli,
        groups: calculate_group_stats(&cases),
        cases,
        suite: SuiteStats {
            average_ms: 100.0,
            case_spread_ms: 0.0,
        },
        warmup_runs: 1,
        measured_iterations: 10,
        thread_count: None,
    };

    let record = to_local_record(&run, None);
    let json = format_record_as_jsonl(&record);
    let parsed = read_record_line(&json);

    assert_eq!(parsed.cases[0].stage_timings[0].name, "ast_ms");
    assert_eq!(parsed.cases[0].stage_timings[0].value, 20.5);
    assert_eq!(
        parsed.cases[0].counters[0].name,
        "StringTable/full clone count"
    );
    assert_eq!(parsed.cases[0].counters[0].value, 8.0);
}

fn read_record_line(line: &str) -> LocalRunRecord {
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join("bench_history_test_single_record.jsonl");
    let _ = fs::remove_file(&path);

    fs::write(&path, line).unwrap();
    let runs = read_local_runs(&path).unwrap();
    let _ = fs::remove_file(&path);

    runs.into_iter().next().expect("record should parse")
}

#[test]
fn test_thread_identity_suffix_default_is_empty() {
    assert_eq!(thread_identity_suffix(None), "");
}

#[test]
fn test_thread_identity_suffix_fixed_is_labeled() {
    assert_eq!(thread_identity_suffix(Some(1)), " [threads: fixed: 1]");
    assert_eq!(thread_identity_suffix(Some(4)), " [threads: fixed: 4]");
}
