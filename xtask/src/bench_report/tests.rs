use super::*;
use crate::bench_history::{LocalCaseRecord, LocalMetricRecord, LocalRunRecord};
use crate::bench_types::{
    BENCHMARK_PROTOCOL_VERSION, BenchmarkChangeKind, BenchmarkMeasurementIdentity, BenchmarkMetric,
    BenchmarkSystem, GitRevision,
};
use crate::benchmark_manifest::{BenchmarkRunner, CliBenchmarkCommand};
use crate::profile::history::{HistoryCaseRecord, HistoryHotFunction, ProfileHistoryRecord};

#[test]
fn report_handles_no_local_history() {
    let report = calculate_benchmark_report(&[], None);
    let rendered = format_benchmark_report(&report);

    assert!(report.suites.is_empty());
    assert!(rendered.starts_with("Benchmark report: local data only"));
    assert!(rendered.contains("No local benchmark history found."));
}

#[test]
fn report_ignores_dirty_latest_run() {
    let system = test_system("SYSTEM-A");
    let mut dirty_latest = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record("check_core", 42.0, vec![], vec![])],
    );
    dirty_latest.git_dirty = Some(true);

    let clean = run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record("check_core", 40.0, vec![], vec![])],
    );

    let report = calculate_benchmark_report(&[clean, dirty_latest], Some(&system));
    let suite = &report.suites[0];
    assert_eq!(suite.latest_timestamp, "2026-05-01T12:00");
}

#[test]
fn report_ignores_unknown_revision_latest_run() {
    let system = test_system("SYSTEM-A");
    let mut unknown_latest = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record("check_core", 42.0, vec![], vec![])],
    );
    unknown_latest.commit = None;

    let clean = run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record("check_core", 40.0, vec![], vec![])],
    );

    let report = calculate_benchmark_report(&[clean, unknown_latest], Some(&system));
    let suite = &report.suites[0];
    assert_eq!(suite.latest_timestamp, "2026-05-01T12:00");
}

#[test]
fn report_names_missing_current_system_history() {
    let system = test_system("SYSTEM-A");
    let report = calculate_benchmark_report(&[], Some(&system));
    let rendered = format_benchmark_report(&report);

    assert!(rendered.contains("No local benchmark history found for the current system."));
}

#[test]
fn report_includes_only_cli_history_when_only_cli_runs_exist() {
    let system = test_system("SYSTEM-A");
    let runs = vec![run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record("check_core", 42.0, vec![], vec![])],
    )];

    let report = calculate_benchmark_report(&runs, Some(&system));

    assert_eq!(report.suites.len(), 1);
    assert_eq!(report.suites[0].suite_kind, BenchmarkSuiteKind::EndToEndCli);
    assert_eq!(report.suites[0].slowest_cases[0].name, "check_core");
}

#[test]
fn report_includes_only_frontend_history_when_only_frontend_runs_exist() {
    let system = test_system("SYSTEM-A");
    let runs = vec![run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            20.0,
            vec![metric("ast_ms", 12.0)],
            vec![],
        )],
    )];

    let report = calculate_benchmark_report(&runs, Some(&system));

    assert_eq!(report.suites.len(), 1);
    assert_eq!(
        report.suites[0].suite_kind,
        BenchmarkSuiteKind::FrontendPhases
    );
    assert_eq!(report.suites[0].slowest_cases[0].stages[0].name, "ast_ms");
}

#[test]
fn report_handles_missing_counters_from_old_records() {
    let system = test_system("SYSTEM-A");
    let previous = run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            20.0,
            vec![metric("ast_ms", 10.0)],
            vec![],
        )],
    );
    let current = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            24.0,
            vec![metric("ast_ms", 13.0)],
            vec![metric("ast_header_count", 10.0)],
        )],
    );

    let report = calculate_benchmark_report(&[previous, current], Some(&system));
    let suite = &report.suites[0];

    assert_eq!(suite.stage_movements[0].stage_name, "ast_ms");
    assert!(
        suite.ratios.iter().any(|ratio| {
            ratio.name == "frontend.ast/ast_header_count" && ratio.case_id == "docs"
        })
    );
}

#[test]
fn report_calculates_ratios_from_dotted_stage_metrics() {
    let system = test_system("SYSTEM-A");
    let runs = vec![run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            20.0,
            vec![metric("frontend.file_prepare", 8.0)],
            vec![metric("source_file_count", 4.0)],
        )],
    )];

    let report = calculate_benchmark_report(&runs, Some(&system));

    assert!(report.suites[0].ratios.iter().any(|ratio| {
        ratio.name == "frontend.file_prepare/source_file_count" && ratio.case_id == "docs"
    }));
}

#[test]
fn report_formats_counter_movements() {
    let system = test_system("SYSTEM-A");
    let previous = run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            20.0,
            vec![],
            vec![metric("token_count", 100.0)],
        )],
    );
    let current = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            20.0,
            vec![],
            vec![
                metric("token_count", 110.0),
                metric("source_file_count", 3.0),
            ],
        )],
    );

    let report = calculate_benchmark_report(&[previous, current], Some(&system));
    let suite = &report.suites[0];

    let token_movement = suite
        .counter_movements
        .iter()
        .find(|movement| movement.name == "token_count")
        .expect("token_count should move by percentage");
    assert_eq!(format_counter_delta(token_movement), "+10%");

    let new_counter_movement = suite
        .counter_movements
        .iter()
        .find(|movement| movement.name == "source_file_count")
        .expect("new counter should move by absolute delta");
    assert_eq!(format_counter_delta(new_counter_movement), "+3");
}

#[test]
fn report_skips_zero_denominator_ratios() {
    let system = test_system("SYSTEM-A");
    let runs = vec![run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record(
            "docs",
            20.0,
            vec![metric("file_prepare_ms", 8.0)],
            vec![metric("source_file_count", 0.0)],
        )],
    )];

    let report = calculate_benchmark_report(&runs, Some(&system));

    assert!(report.suites[0].ratios.is_empty());
}

#[test]
fn report_flags_unattributed_cli_wall_time() {
    let system = test_system("SYSTEM-A");
    let runs = vec![run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record(
            "check_root_file",
            1_000.0,
            vec![
                metric("command.check.path_validation", 10.0),
                metric("command.check.builder_construction", 5.0),
                metric("command.check.bootstrap", 20.0),
                metric("command.check.compile_project_frontend", 15.0),
                metric("command.check.message_rendering", 2.0),
            ],
            vec![],
        )],
    )];

    let report = calculate_benchmark_report(&runs, Some(&system));
    let rendered = format_benchmark_report(&report);

    assert_eq!(report.suites[0].unattributed_cases.len(), 1);
    assert_eq!(
        report.suites[0].unattributed_cases[0].name,
        "check_root_file"
    );
    assert!(rendered.contains("Unattributed wall time:"));
    assert!(rendered.contains("check_root_file"));
}

#[test]
fn report_flags_unattributed_build_wall_time() {
    let system = test_system("SYSTEM-A");
    let runs = vec![run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::EndToEndCli,
        "SYSTEM-A",
        vec![case_record(
            "build_module_root",
            1_000.0,
            vec![
                metric("build_project.path_validation", 5.0),
                metric("build_project.bootstrap", 30.0),
                metric("build_project.compile_project_frontend", 200.0),
                metric("build_project.backend", 150.0),
                metric("command.build.output_write", 20.0),
                // Nested totals that must NOT be summed to avoid double-counting.
                metric("build_project.total", 405.0),
                metric("command.build.total", 425.0),
                metric("output.write_total", 19.5),
            ],
            vec![],
        )],
    )];

    let report = calculate_benchmark_report(&runs, Some(&system));
    let rendered = format_benchmark_report(&report);

    // Attributed sum = 5 + 30 + 200 + 150 + 20 = 405ms; wall = 1000ms;
    // unattributed = 595ms which exceeds the 100ms threshold.
    assert_eq!(report.suites[0].unattributed_cases.len(), 1);
    assert_eq!(
        report.suites[0].unattributed_cases[0].name,
        "build_module_root"
    );
    let unattributed = &report.suites[0].unattributed_cases[0];
    assert!((unattributed.unattributed_ms - 595.0).abs() < 0.01);
    assert!((unattributed.unattributed_ratio - 0.595).abs() < 0.001);
    assert!(rendered.contains("Unattributed wall time:"));
    assert!(rendered.contains("build_module_root"));
}

#[test]
fn report_uses_latest_per_suite_without_system_identity() {
    let runs = vec![
        run_record(
            "2026-05-01T12:00",
            BenchmarkSuiteKind::EndToEndCli,
            "SYSTEM-A",
            vec![case_record("old", 50.0, vec![], vec![])],
        ),
        run_record(
            "2026-05-02T12:00",
            BenchmarkSuiteKind::EndToEndCli,
            "SYSTEM-B",
            vec![case_record("new", 30.0, vec![], vec![])],
        ),
    ];

    let report = calculate_benchmark_report(&runs, None);
    let rendered = format_benchmark_report(&report);

    assert_eq!(report.suites.len(), 1);
    assert_eq!(report.suites[0].slowest_cases[0].name, "new");
    assert!(rendered.contains("No local system identity found"));
}

fn test_system(system_uuid: &str) -> BenchmarkSystem {
    BenchmarkSystem {
        system_uuid: system_uuid.to_string(),
        public_system_id: "ABC123".to_string(),
        display_name: "Test System".to_string(),
    }
}

fn run_record(
    timestamp: &str,
    suite_kind: BenchmarkSuiteKind,
    system_uuid: &str,
    cases: Vec<LocalCaseRecord>,
) -> LocalRunRecord {
    LocalRunRecord {
        format_version: 6,
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        timestamp: timestamp.to_string(),
        month_key: "2026-05".to_string(),
        commit: Some("abc1234".to_string()),
        git_dirty: Some(false),
        system_uuid: system_uuid.to_string(),
        public_system_id: "ABC123".to_string(),
        display_name: "Test System".to_string(),
        warmup_runs: 1,
        measured_iterations: 10,
        suite_kind: suite_kind.persisted_name().to_string(),
        primary_metric_name: suite_kind.primary_metric_name().to_string(),
        suite_average_ms: 0.0,
        suite_case_spread_ms: 0.0,
        thread_count: None,
        groups: vec![],
        cases,
    }
}

fn case_record(
    name: &str,
    mean_ms: f64,
    stage_timings: Vec<LocalMetricRecord>,
    counters: Vec<LocalMetricRecord>,
) -> LocalCaseRecord {
    LocalCaseRecord {
        case_id: name.to_string(),
        workload_id: Some(format!("{name}_workload")),
        source_fingerprint: Some(format!("{name}_source_fp")),
        measurement_fingerprint: Some(format!("{name}_measurement_fp")),
        group_name: "test".to_string(),
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: vec![name.to_string()],
        },
        mean_ms,
        median_ms: mean_ms,
        stddev_ms: 0.0,
        stage_timings,
        counters,
    }
}

fn metric(name: &str, value: f64) -> LocalMetricRecord {
    LocalMetricRecord {
        name: name.to_string(),
        value,
    }
}

// ---------------------------------------------------------------------------
//  Latest profile run tests
// ---------------------------------------------------------------------------

/// Build a minimal `ProfileHistoryRecord` for testing.
fn test_profile_record(run_id: &str, system_uuid: &str) -> ProfileHistoryRecord {
    ProfileHistoryRecord {
        format_version: 4,
        profile_protocol_version: crate::profile::history::PROFILE_PROTOCOL_VERSION,
        run_id: run_id.to_string(),
        timestamp: "June 18th - 10:30".to_string(),
        git_revision: GitRevision {
            commit: Some("abc1234".to_string()),
            dirty: Some(false),
        },
        system_uuid: system_uuid.to_string(),
        system_display: "Test System".to_string(),
        filter_mode: "terse".to_string(),
        sample_rate_hz: None,
        cases: vec![HistoryCaseRecord {
            case_id: "check_foo_bst".to_string(),
            identity: test_identity(),
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
                inclusive_samples: 400.0,
                self_samples: 200.0,
                inclusive_pct: 30.0,
                self_pct: 16.0,
            }],
            top_bucket_label: "AST".to_string(),
            run_directory_path: format!("benchmarks/local-data/profiles/{}", run_id),
        }],
    }
}

/// Build a test identity for drift comparison.
fn test_identity() -> BenchmarkMeasurementIdentity {
    BenchmarkMeasurementIdentity {
        workload_id: "foo".to_string(),
        source_fingerprint: "aaaa1111aaaa1111".to_string(),
        measurement_fingerprint: "bbbb2222bbbb2222".to_string(),
    }
}

/// Build a second profile record with different hotspot data for drift testing.
fn test_profile_record_shifted(run_id: &str, system_uuid: &str) -> ProfileHistoryRecord {
    ProfileHistoryRecord {
        format_version: 4,
        profile_protocol_version: crate::profile::history::PROFILE_PROTOCOL_VERSION,
        run_id: run_id.to_string(),
        timestamp: "June 18th - 11:00".to_string(),
        git_revision: GitRevision {
            commit: Some("def5678".to_string()),
            dirty: Some(false),
        },
        system_uuid: system_uuid.to_string(),
        system_display: "Test System".to_string(),
        filter_mode: "terse".to_string(),
        sample_rate_hz: None,
        cases: vec![HistoryCaseRecord {
            case_id: "check_foo_bst".to_string(),
            identity: test_identity(),
            group_name: "core".to_string(),
            command: "check".to_string(),
            args: vec!["foo.moth".to_string()],
            observation_wall_ms: 1500.0,
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
                inclusive_samples: 500.0,
                self_samples: 250.0,
                inclusive_pct: 41.7,
                self_pct: 20.0,
            }],
            top_bucket_label: "AST".to_string(),
            run_directory_path: format!("benchmarks/local-data/profiles/{}", run_id),
        }],
    }
}

#[test]
fn report_has_no_latest_profile_run_section_when_none() {
    let mut report = calculate_benchmark_report(&[], None);
    report.latest_profile_run = None;
    let rendered = format_benchmark_report(&report);

    assert!(!rendered.contains("Latest profile run"));
}

#[test]
fn report_includes_latest_profile_run_section() {
    let mut report = calculate_benchmark_report(&[], None);
    report.latest_profile_run = Some(LatestProfileRun {
        run_id: "2026-06-18T10-30-abc1234".to_string(),
        filter_mode: "terse".to_string(),
        case_count: 3,
        top_drift_item: "none".to_string(),
        agent_summary_path:
            "benchmarks/local-data/profiles/2026-06-18T10-30-abc1234/agent-summary.md".to_string(),
        legacy_run: None,
    });
    let rendered = format_benchmark_report(&report);

    assert!(rendered.contains("Latest profile run:"));
    assert!(rendered.contains("2026-06-18T10-30-abc1234"));
    assert!(rendered.contains("Filter:    terse"));
    assert!(rendered.contains("Cases:     3"));
    assert!(rendered.contains("Top drift: none"));
    assert!(rendered.contains("agent-summary.md"));
}

#[test]
fn format_top_drift_item_shows_drift_when_comparable_previous_exists() {
    let previous = test_profile_record("2026-06-18T10-00-old0001", "TEST-UUID-001");
    let latest = test_profile_record_shifted("2026-06-18T11-00-new0002", "TEST-UUID-001");
    let records: Vec<StoredProfileHistoryRecord> = vec![
        StoredProfileHistoryRecord::Current(previous.clone()),
        StoredProfileHistoryRecord::Current(latest.clone()),
    ];

    // Debug: verify find_comparable_previous finds the previous record.
    let found = crate::profile::drift::find_comparable_previous(
        &records,
        "TEST-UUID-001",
        "terse",
        None,
        "2026-06-18T11-00-new0002",
    );
    assert!(
        found.is_some(),
        "find_comparable_previous should find the previous record"
    );
    assert_eq!(found.unwrap().run_id, "2026-06-18T10-00-old0001");

    // Debug: verify compute_drift finds the function drift.
    let drift_cases = vec![crate::profile::drift::DriftCaseInput {
        case_id: "check_foo_bst".to_string(),
        identity: test_identity(),
        command: "check".to_string(),
        args: vec!["foo.moth".to_string()],
        stage_timings: vec![BenchmarkMetric {
            name: "ast_ms".to_string(),
            value: 900.0,
        }],
        counters: vec![BenchmarkMetric {
            name: "token_count".to_string(),
            value: 13000.0,
        }],
        hot_functions: vec![crate::profile::drift::DriftHotFunction {
            name: "moth::compiler_frontend::ast::resolve_type".to_string(),
            bucket_label: "AST".to_string(),
            inclusive_samples: 500.0,
            inclusive_pct: 41.7,
        }],
    }];
    let wall_times = std::collections::HashMap::from([("check_foo_bst".to_string(), 1500.0)]);
    let drift_report =
        crate::profile::drift::compute_drift(&drift_cases, found.unwrap(), &wall_times);

    assert!(
        !drift_report.function_increases.is_empty(),
        "compute_drift should find function increases; decreases={}, stages={}, counters={}",
        drift_report.function_decreases.len(),
        drift_report.stage_movements.len(),
        drift_report.counter_movements.len()
    );

    // Now test the full format_top_drift_item path.
    let system = test_system("TEST-UUID-001");
    let top_drift = format_top_drift_item(&records, Some(&system), &latest);

    // The shifted record has resolve_type at 41.7% vs 30.0%, a +11.7pp increase.
    assert!(
        top_drift.contains("+11.7pp"),
        "expected significant drift, got: {}",
        top_drift
    );
    assert!(
        top_drift.contains("resolve_type"),
        "expected function name, got: {}",
        top_drift
    );
    assert!(
        top_drift.contains("AST"),
        "expected bucket label, got: {}",
        top_drift
    );
}

#[test]
fn format_top_drift_item_returns_none_when_no_comparable_previous() {
    let latest = test_profile_record("2026-06-18T10-30-abc1234", "TEST-UUID-001");
    let records = vec![StoredProfileHistoryRecord::Current(latest.clone())];

    let system = test_system("TEST-UUID-001");
    let top_drift = format_top_drift_item(&records, Some(&system), &latest);

    assert_eq!(top_drift, "none");
}

// ---------------------------------------------------------------------------
//  Thread identity tests
// ---------------------------------------------------------------------------

#[test]
fn report_selects_only_matching_thread_identity_for_comparison() {
    let system = test_system("SYSTEM-A");

    // Previous run with default threads, latest run with fixed 4 threads.
    // The report must NOT compare them because thread identities differ.
    let previous = run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record("docs", 20.0, vec![], vec![])],
    );
    let mut current = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record("docs", 24.0, vec![], vec![])],
    );
    current.thread_count = Some(4);

    let report = calculate_benchmark_report(&[previous, current], Some(&system));
    let suite = &report.suites[0];

    // No comparable previous run exists for thread identity Some(4),
    // so the comparison must be a baseline.
    assert_eq!(suite.comparison.change_kind, BenchmarkChangeKind::Baseline);
    assert_eq!(suite.comparison.compared_case_count, 0);
}

#[test]
fn report_compares_runs_with_same_thread_identity() {
    let system = test_system("SYSTEM-A");

    let mut previous = run_record(
        "2026-05-01T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record("docs", 20.0, vec![], vec![])],
    );
    previous.thread_count = Some(2);

    let mut current = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record("docs", 24.0, vec![], vec![])],
    );
    current.thread_count = Some(2);

    let report = calculate_benchmark_report(&[previous, current], Some(&system));
    let suite = &report.suites[0];

    // Same thread identity: the report must find a comparable previous run.
    assert_ne!(suite.comparison.change_kind, BenchmarkChangeKind::Baseline);
    assert_eq!(suite.comparison.compared_case_count, 1);
}

#[test]
fn report_renders_fixed_thread_identity_in_header() {
    let system = test_system("SYSTEM-A");

    let mut current = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record("docs", 24.0, vec![], vec![])],
    );
    current.thread_count = Some(4);

    let report = calculate_benchmark_report(&[current], Some(&system));
    let rendered = format_benchmark_report(&report);

    assert!(
        rendered.contains("Threads: fixed: 4"),
        "report should show fixed thread identity: got\n{rendered}"
    );
}

#[test]
fn report_renders_default_thread_identity_in_header() {
    let system = test_system("SYSTEM-A");

    let current = run_record(
        "2026-05-02T12:00",
        BenchmarkSuiteKind::FrontendPhases,
        "SYSTEM-A",
        vec![case_record("docs", 24.0, vec![], vec![])],
    );

    let report = calculate_benchmark_report(&[current], Some(&system));
    let rendered = format_benchmark_report(&report);

    assert!(
        rendered.contains("Threads: default"),
        "report should show default thread identity: got\n{rendered}"
    );
}
