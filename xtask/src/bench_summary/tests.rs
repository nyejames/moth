use super::*;
use crate::bench_history::{LocalCaseRecord, LocalGroupRecord, LocalRunRecord};
use crate::bench_time::BenchmarkTimestamp;
use crate::bench_types::{
    BENCHMARK_PROTOCOL_VERSION, BenchmarkCaseObservations, BenchmarkCaseResult,
    BenchmarkChangeKind, BenchmarkComparison, BenchmarkMeasurementIdentity, BenchmarkMetric,
    BenchmarkRun, BenchmarkSuiteKind, BenchmarkSystem, GitRevision, SuiteStats,
    calculate_group_stats,
};
use crate::benchmark_manifest::{BenchmarkRunner, CliBenchmarkCommand};

fn benchmark_case(case_name: &str, mean_ms: f64) -> BenchmarkCaseResult {
    benchmark_group_case(case_name, "ungrouped", mean_ms)
}

fn benchmark_group_case(case_name: &str, group_name: &str, mean_ms: f64) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        case_id: case_name.to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: format!("{case_name}_workload"),
            source_fingerprint: format!("{case_name}_source_fp"),
            measurement_fingerprint: format!("{case_name}_measurement_fp"),
        }),
        group_name: group_name.to_string(),
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
        mean_ms,
        median_ms: mean_ms,
        stddev_ms: 0.0,
        observations: Default::default(),
    }
}

fn local_record_from_cases(cases: Vec<BenchmarkCaseResult>) -> LocalRunRecord {
    let groups = calculate_group_stats(&cases);
    let suite = SuiteStats::from_case_results(&cases);

    LocalRunRecord {
        format_version: 6,
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: "wall_time_ms".to_string(),
        timestamp: "2026-05-10T15:21".to_string(),
        month_key: "2026-05".to_string(),
        commit: Some("abc123".to_string()),
        git_dirty: Some(false),
        system_uuid: "UUID123".to_string(),
        public_system_id: "B7F2A9".to_string(),
        display_name: "macOS M1".to_string(),
        warmup_runs: 1,
        measured_iterations: 10,
        suite_average_ms: suite.average_ms,
        suite_case_spread_ms: suite.case_spread_ms,
        thread_count: None,
        groups: groups
            .into_iter()
            .map(|group| LocalGroupRecord {
                name: group.group_name,
                case_count: group.case_count,
                average_ms: group.average_ms,
            })
            .collect(),
        cases: cases
            .into_iter()
            .map(|case| LocalCaseRecord {
                case_id: case.case_id,
                workload_id: case.identity.as_ref().map(|id| id.workload_id.clone()),
                source_fingerprint: case
                    .identity
                    .as_ref()
                    .map(|id| id.source_fingerprint.clone()),
                measurement_fingerprint: case
                    .identity
                    .as_ref()
                    .map(|id| id.measurement_fingerprint.clone()),
                group_name: case.group_name,
                runner: case.runner,
                mean_ms: case.mean_ms,
                median_ms: case.median_ms,
                stddev_ms: case.stddev_ms,
                stage_timings: Vec::new(),
                counters: Vec::new(),
            })
            .collect(),
    }
}

fn local_record(average_ms: f64, case_spread_ms: f64) -> LocalRunRecord {
    LocalRunRecord {
        format_version: 6,
        benchmark_protocol_version: BENCHMARK_PROTOCOL_VERSION,
        timestamp: "2026-05-10T15:21".to_string(),
        month_key: "2026-05".to_string(),
        commit: Some("abc123".to_string()),
        git_dirty: Some(false),
        system_uuid: "UUID123".to_string(),
        public_system_id: "B7F2A9".to_string(),
        display_name: "macOS M1".to_string(),
        warmup_runs: 1,
        measured_iterations: 10,
        suite_kind: "end_to_end_cli".to_string(),
        primary_metric_name: "wall_time_ms".to_string(),
        suite_average_ms: average_ms,
        suite_case_spread_ms: case_spread_ms,
        thread_count: None,
        groups: vec![
            LocalGroupRecord {
                name: "core".to_string(),
                case_count: 2,
                average_ms: average_ms + 40.0,
            },
            LocalGroupRecord {
                name: "docs".to_string(),
                case_count: 1,
                average_ms: average_ms - 20.0,
            },
        ],
        cases: vec![LocalCaseRecord {
            case_id: "check_docs".to_string(),
            workload_id: Some("docs".to_string()),
            source_fingerprint: Some("docs_source_fp".to_string()),
            measurement_fingerprint: Some("docs_measurement_fp".to_string()),
            group_name: "docs".to_string(),
            runner: BenchmarkRunner::Cli {
                command: CliBenchmarkCommand::Check,
                args: Vec::new(),
            },
            mean_ms: average_ms,
            median_ms: average_ms,
            stddev_ms: 1.0,
            stage_timings: Vec::new(),
            counters: Vec::new(),
        }],
    }
}

fn benchmark_run(cases: Vec<BenchmarkCaseResult>) -> BenchmarkRun {
    let groups = calculate_group_stats(&cases);
    let suite = SuiteStats::from_case_results(&cases);

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
        cases,
        groups,
        suite,
        warmup_runs: 1,
        measured_iterations: 10,
        thread_count: None,
    }
}

#[test]
fn test_round_ms() {
    assert_eq!(round_ms(68.4), 68);
    assert_eq!(round_ms(68.5), 69);
    assert_eq!(round_ms(68.6), 69);
    assert_eq!(round_ms(-10.4), -10);
    assert_eq!(round_ms(-10.5), -11); // Rust f64::round rounds half away from zero
    assert_eq!(round_ms(-10.6), -11);
    assert_eq!(round_ms(0.0), 0);
}

#[test]
fn test_format_delta_line() {
    assert_eq!(format_delta_line(-12.4), "-12ms");
    assert_eq!(format_delta_line(8.5), "+9ms");
    assert_eq!(format_delta_line(0.0), "0ms");
    assert_eq!(format_delta_line(-0.4), "0ms");
}

#[test]
fn test_format_average_and_case_spread() {
    assert_eq!(format_average_ms(68.0), "~68ms");
    assert_eq!(format_average_ms(80.4), "~80ms");
    assert_eq!(format_case_spread_ms(9.0), "~9ms");
}

#[test]
fn test_format_group_average_line() {
    let cases = vec![
        BenchmarkCaseResult {
            case_id: "a".to_string(),
            identity: Some(BenchmarkMeasurementIdentity {
                workload_id: "a_workload".to_string(),
                source_fingerprint: "a_source_fp".to_string(),
                measurement_fingerprint: "a_measurement_fp".to_string(),
            }),
            group_name: "core".to_string(),
            runner: BenchmarkRunner::Cli {
                command: CliBenchmarkCommand::Check,
                args: Vec::new(),
            },
            mean_ms: 100.0,
            median_ms: 100.0,
            stddev_ms: 0.0,
            observations: Default::default(),
        },
        BenchmarkCaseResult {
            case_id: "b".to_string(),
            identity: Some(BenchmarkMeasurementIdentity {
                workload_id: "b_workload".to_string(),
                source_fingerprint: "b_source_fp".to_string(),
                measurement_fingerprint: "b_measurement_fp".to_string(),
            }),
            group_name: "docs".to_string(),
            runner: BenchmarkRunner::Cli {
                command: CliBenchmarkCommand::Check,
                args: Vec::new(),
            },
            mean_ms: 50.0,
            median_ms: 50.0,
            stddev_ms: 0.0,
            observations: Default::default(),
        },
    ];
    let suite = SuiteStats::from_case_results(&cases);
    let groups = calculate_group_stats(&cases);

    assert_eq!(
        format_group_average_line(&suite, &groups),
        "Avg: all ~75ms, core ~100ms, docs ~50ms"
    );
}

#[test]
fn test_generate_system_block_with_change() {
    let initial = local_record(80.0, 6.0);
    let latest = local_record(68.0, 9.0);
    let block = generate_system_block("End-to-end CLI", "macOS M1", "B7F2A9", &initial, &latest, 2);

    assert!(block.contains("## End-to-end CLI / macOS M1 (B7F2A9)"));
    assert!(
        block.contains("Change since initial benchmark: -12ms avg; 1 faster, 0 slower; 1/1 cases")
    );
    assert!(block.contains("Initial: all ~80ms, core ~120ms, docs ~60ms"));
    assert!(block.contains("Latest: all ~68ms, core ~108ms, docs ~48ms"));
    assert!(block.contains("Case spread latest: ~9ms"));
}

#[test]
fn test_generate_system_block_baseline_single_run() {
    let initial = local_record(80.0, 6.0);
    let latest = local_record(80.0, 6.0);
    // run_count == 1 should show baseline even with zero change
    let block = generate_system_block("End-to-end CLI", "macOS M1", "B7F2A9", &initial, &latest, 1);

    assert!(block.contains("Change since initial benchmark: baseline"));
}

#[test]
fn test_generate_system_block_two_runs_zero_change() {
    let initial = local_record(80.0, 6.0);
    let latest = local_record(80.0, 6.0);
    let block = generate_system_block("End-to-end CLI", "macOS M1", "B7F2A9", &initial, &latest, 2);

    assert!(
        block.contains("Change since initial benchmark: no measurable change: avg 0ms; 1/1 cases")
    );
}

#[test]
fn test_generate_system_block_two_runs_small_change() {
    let initial = local_record(80.0, 6.0);
    let latest = local_record(80.3, 6.0);
    let block = generate_system_block("End-to-end CLI", "macOS M1", "B7F2A9", &initial, &latest, 2);

    assert!(
        block.contains("Change since initial benchmark: no measurable change: avg 0ms; 1/1 cases")
    );
}

#[test]
fn test_generate_system_block_reports_case_set_changed() {
    let initial = local_record_from_cases(vec![
        benchmark_group_case("check_core", "core", 82.0),
        benchmark_group_case("check_docs", "docs", 87.0),
        benchmark_group_case("check_stress", "stress", 8.0),
    ]);
    let latest = local_record_from_cases(vec![
        benchmark_group_case("check_core", "core", 105.0),
        benchmark_group_case("check_docs", "docs", 105.0),
        benchmark_group_case("check_stress", "stress", 11.0),
        benchmark_group_case("check_module", "module", 11.0),
        benchmark_group_case("check_borrow", "borrow", 12.0),
    ]);

    let block = generate_system_block("End-to-end CLI", "macOS M1", "B7F2A9", &initial, &latest, 2);

    assert!(block.contains("Change since initial benchmark: case set changed"));
    assert!(!block.contains("Change since initial benchmark: 0ms avg"));
    assert!(block.contains(
        "Latest: all ~49ms, core ~105ms, docs ~105ms, stress ~11ms, module ~11ms, borrow ~12ms"
    ));
}

#[test]
fn test_generate_system_block_same_case_set_reports_counts() {
    let initial = local_record_from_cases(vec![
        benchmark_group_case("check_core", "core", 100.0),
        benchmark_group_case("check_docs", "docs", 80.0),
    ]);
    let latest = local_record_from_cases(vec![
        benchmark_group_case("check_core", "core", 110.0),
        benchmark_group_case("check_docs", "docs", 90.0),
    ]);

    let block = generate_system_block("End-to-end CLI", "macOS M1", "B7F2A9", &initial, &latest, 2);

    assert!(
        block.contains("Change since initial benchmark: +10ms avg; 0 faster, 2 slower; 2/2 cases")
    );
    assert!(!block.contains("case set changed"));
}

#[test]
fn test_generate_run_entry_with_delta() {
    let cases = vec![benchmark_case("a", 120.0)];
    let current = vec![benchmark_case("a", 110.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&cases));

    let run = benchmark_run(current);
    let entry = generate_run_entry(&run, &comparison);
    assert_eq!(entry.display_name, "macOS M1");
    assert_eq!(entry.public_system_id, "B7F2A9");
    assert_eq!(entry.timestamp_text, "May 10th - 15:21");
    assert_eq!(
        entry.body,
        "**-10ms avg**; 1 faster, 0 slower; 1/1 cases\nAvg: all ~110ms, ungrouped ~110ms"
    );
    assert!(
        entry
            .to_markdown()
            .contains("# End-to-end CLI / macOS M1 (B7F2A9): May 10th - 15:21")
    );
}

#[test]
fn test_generate_run_entry_baseline() {
    let current = vec![benchmark_case("a", 110.0)];
    let comparison = BenchmarkComparison::new(&current, None);
    let run = benchmark_run(current);

    let entry = generate_run_entry(&run, &comparison);
    assert_eq!(
        entry.body,
        "**baseline**; 1 cases, avg ~110ms\nAvg: all ~110ms, ungrouped ~110ms"
    );
}

#[test]
fn test_generate_run_entry_with_stage_movement() {
    let current = vec![BenchmarkCaseResult {
        case_id: "a".to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: "a_workload".to_string(),
            source_fingerprint: "a_source_fp".to_string(),
            measurement_fingerprint: "a_measurement_fp".to_string(),
        }),
        group_name: "ungrouped".to_string(),
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
        mean_ms: 110.0,
        median_ms: 110.0,
        stddev_ms: 0.0,
        observations: BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 55.0,
            }],
            counters: vec![],
        },
    }];
    let previous = vec![BenchmarkCaseResult {
        case_id: "a".to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: "a_workload".to_string(),
            source_fingerprint: "a_source_fp".to_string(),
            measurement_fingerprint: "a_measurement_fp".to_string(),
        }),
        group_name: "ungrouped".to_string(),
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
        mean_ms: 100.0,
        median_ms: 100.0,
        stddev_ms: 0.0,
        observations: BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 50.0,
            }],
            counters: vec![],
        },
    }];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));
    let run = benchmark_run(current);

    let entry = generate_run_entry(&run, &comparison);
    assert!(entry.body.contains("Stage movement:"));
    assert!(entry.body.contains("ast +5ms"));
}

#[test]
fn test_generate_run_entry_baseline_hides_stage_movement() {
    let current = vec![BenchmarkCaseResult {
        case_id: "a".to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: "a_workload".to_string(),
            source_fingerprint: "a_source_fp".to_string(),
            measurement_fingerprint: "a_measurement_fp".to_string(),
        }),
        group_name: "ungrouped".to_string(),
        runner: BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
        mean_ms: 110.0,
        median_ms: 110.0,
        stddev_ms: 0.0,
        observations: BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "ast_ms".to_string(),
                value: 55.0,
            }],
            counters: vec![],
        },
    }];
    let comparison = BenchmarkComparison::new(&current, None);
    let run = benchmark_run(current);

    let entry = generate_run_entry(&run, &comparison);
    assert!(!entry.body.contains("Stage movement:"));
}

#[test]
fn test_generate_run_entry_no_stage_data_hides_movement() {
    let current = vec![benchmark_case("a", 110.0)];
    let previous = vec![benchmark_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));
    let run = benchmark_run(current);

    let entry = generate_run_entry(&run, &comparison);
    assert!(!entry.body.contains("Stage movement:"));
}

#[test]
fn test_parse_existing_summary_new_format() {
    let content = r#"# May 2026 Summary

## macOS M1 (B7F2A9)
Change since initial benchmark: -12ms
Initial benchmark: ~80ms (+/- 6ms)
Latest benchmark: ~68ms (+/- 9ms)

## Linux x64 (A91C)
Change since initial benchmark: baseline
Initial benchmark: ~120ms (+/- 11ms)
Latest benchmark: ~120ms (+/- 11ms)

---------------------

# macOS M1 (B7F2A9): May 10th - 15:21
**baseline** (+/- 0ms)

"#;

    let (other_blocks, run_entries) = parse_existing_summary(content, "B7F2A9", "End-to-end CLI");

    assert_eq!(other_blocks.len(), 1);
    assert!(other_blocks[0].contains("Linux x64 (A91C)"));
    assert!(!other_blocks[0].contains("macOS M1"));

    assert_eq!(run_entries.len(), 1);
    assert!(
        run_entries[0]
            .to_markdown()
            .contains("# End-to-end CLI / macOS M1 (B7F2A9): May 10th - 15:21")
    );
    assert!(
        run_entries[0]
            .to_markdown()
            .contains("**baseline** (+/- 0ms)")
    );
}

#[test]
fn test_parse_existing_summary_old_format_backward_compat() {
    let content = r#"# May 2026 Summary

<!-- BENCHMARK_MONTH_SUMMARY_START -->
## macOS M1 (B7F2A9)
Change since initial benchmark: -12ms
Initial benchmark: ~80ms (+/- 6ms)
Latest benchmark: ~68ms (+/- 9ms)

## Linux x64 (A91C)
Change since initial benchmark: baseline
Initial benchmark: ~120ms (+/- 11ms)
Latest benchmark: ~120ms (+/- 11ms)
<!-- BENCHMARK_MONTH_SUMMARY_END -->

<!-- BENCHMARK_RUNS_START -->

# macOS M1 (B7F2A9): May 10th - 15:21
**baseline** (+/- 0ms)
"#;

    let (other_blocks, run_entries) = parse_existing_summary(content, "B7F2A9", "End-to-end CLI");

    assert_eq!(other_blocks.len(), 1);
    assert!(other_blocks[0].contains("Linux x64 (A91C)"));
    assert!(!other_blocks[0].contains("macOS M1"));

    assert_eq!(run_entries.len(), 1);
    assert!(
        run_entries[0]
            .to_markdown()
            .contains("# End-to-end CLI / macOS M1 (B7F2A9): May 10th - 15:21")
    );
    assert!(
        run_entries[0]
            .to_markdown()
            .contains("**baseline** (+/- 0ms)")
    );
}

#[test]
fn test_parse_existing_summary_fallback() {
    // Unrecognized format without markers or separator — degrades gracefully
    let content = r#"# May 2026 Summary

## macOS M1 (B7F2A9)
Change since initial benchmark: -12ms
Initial benchmark: ~80ms (+/- 6ms)
Latest benchmark: ~68ms (+/- 9ms)

# macOS M1 (B7F2A9): May 10th - 15:21
**baseline** (+/- 0ms)
"#;

    let (other_blocks, run_entries) = parse_existing_summary(content, "B7F2A9", "End-to-end CLI");

    assert!(other_blocks.is_empty());
    assert!(run_entries.is_empty());
}

#[test]
fn test_build_summary_content_new_file() {
    let block = "## macOS M1 (B7F2A9)\nChange: baseline\n".to_string();
    let entries = vec![ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 10th - 15:21".to_string(),
        body: "**baseline** (+/- 0ms)".to_string(),
        raw: "# macOS M1 (B7F2A9): May 10th - 15:21\n**baseline** (+/- 0ms)\n".to_string(),
    })];

    let content = build_summary_content("May 2026", &[block], &entries);

    assert!(content.starts_with("# May 2026 Summary\n\n"));
    assert!(!content.contains("<!--"));
    assert!(content.contains("---------------------"));
    assert!(content.contains("## macOS M1 (B7F2A9)"));
    assert!(content.contains("# End-to-end CLI / macOS M1 (B7F2A9): May 10th - 15:21"));
}

#[test]
fn test_build_summary_content_appends_run() {
    let block = "## macOS M1 (B7F2A9)\nChange: -12ms\n".to_string();
    let entries = vec![
        ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
            suite_kind_label: "End-to-end CLI".to_string(),
            display_name: "macOS M1".to_string(),
            public_system_id: "B7F2A9".to_string(),
            timestamp_text: "May 10th - 15:21".to_string(),
            body: "**baseline** (+/- 0ms)".to_string(),
            raw: "# macOS M1 (B7F2A9): May 10th - 15:21\n**baseline** (+/- 0ms)\n".to_string(),
        }),
        ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
            suite_kind_label: "End-to-end CLI".to_string(),
            display_name: "macOS M1".to_string(),
            public_system_id: "B7F2A9".to_string(),
            timestamp_text: "May 10th - 16:04".to_string(),
            body: "**-10ms** (+/- 5ms)".to_string(),
            raw: "# macOS M1 (B7F2A9): May 10th - 16:04\n**-10ms** (+/- 5ms)\n".to_string(),
        }),
    ];

    let content = build_summary_content("May 2026", &[block], &entries);

    // Should contain both run entries
    assert!(content.contains("May 10th - 15:21"));
    assert!(content.contains("May 10th - 16:04"));
    // And the updated summary block
    assert!(content.contains("Change: -12ms"));
    // Each entry should have a blank line after it
    let runs_start = content.find("---------------------").unwrap();
    let runs_section = &content[runs_start..];
    // Two entries means two blank-line-separated blocks in the runs section
    assert!(runs_section.contains("**baseline** (+/- 0ms)\n\n# End-to-end CLI / macOS M1"));
}

#[test]
fn test_build_summary_content_multiple_systems() {
    let blocks = vec![
        "## macOS M1 (B7F2A9)\nChange: -12ms\n".to_string(),
        "## Linux x64 (A91C)\nChange: baseline\n".to_string(),
    ];
    let entries = vec![ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 10th - 15:21".to_string(),
        body: "**baseline** (+/- 0ms)".to_string(),
        raw: "# macOS M1 (B7F2A9): May 10th - 15:21\n**baseline** (+/- 0ms)\n".to_string(),
    })];

    let content = build_summary_content("May 2026", &blocks, &entries);

    assert!(content.contains("## macOS M1 (B7F2A9)"));
    assert!(content.contains("## Linux x64 (A91C)"));
}

#[test]
fn test_parse_run_entries_splits_on_headings() {
    let text = r#"# macOS M1 (B7F2A9): May 10th - 15:21
**baseline** (+/- 0ms)

# macOS M1 (B7F2A9): May 10th - 16:04
**-10ms** (+/- 5ms)

"#;

    let entries = parse_run_entries(text);

    assert_eq!(entries.len(), 2);
    assert!(entries[0].to_markdown().contains("May 10th - 15:21"));
    assert!(entries[0].to_markdown().contains("**baseline** (+/- 0ms)"));
    assert!(entries[1].to_markdown().contains("May 10th - 16:04"));
    assert!(entries[1].to_markdown().contains("**-10ms** (+/- 5ms)"));
}

#[test]
fn test_parse_run_entries_empty() {
    let entries = parse_run_entries("");
    assert!(entries.is_empty());
}

#[test]
fn test_parse_run_entries_single() {
    let text = "# macOS M1 (B7F2A9): May 10th - 15:21\n**baseline** (+/- 0ms)\n";
    let entries = parse_run_entries(text);

    assert_eq!(entries.len(), 1);
    assert!(entries[0].to_markdown().contains("May 10th - 15:21"));
}

#[test]
fn test_parse_run_entry_numeric() {
    let raw = "# macOS M1 (B7F2A9): May 10th - 15:21\n**-10ms** (+/- 5ms)";
    let entry = parse_run_entry(raw).unwrap();
    assert_eq!(entry.display_name, "macOS M1");
    assert_eq!(entry.public_system_id, "B7F2A9");
    assert_eq!(entry.timestamp_text, "May 10th - 15:21");
    assert_eq!(entry.body, "**-10ms** (+/- 5ms)");
}

#[test]
fn test_parse_run_entry_baseline() {
    let raw = "# macOS M1 (B7F2A9): May 10th - 15:21\n**baseline** (+/- 0ms)";
    let entry = parse_run_entry(raw).unwrap();
    assert_eq!(entry.body, "**baseline** (+/- 0ms)");
}

#[test]
fn test_parse_run_entry_no_change() {
    let raw = "# macOS M1 (B7F2A9): May 10th - 15:21\nno measurable change since last benchmark";
    let entry = parse_run_entry(raw).unwrap();
    assert_eq!(entry.body, "no measurable change since last benchmark");
    assert!(is_no_measurable_change_entry(&entry));
}

#[test]
fn test_parse_run_entry_preserves_malformed() {
    let raw = "not a heading\nsome random text";
    assert!(parse_run_entry(raw).is_none());
}

#[test]
fn test_parse_run_entries_preserves_malformed_raw() {
    let text = r#"# macOS M1 (B7F2A9): May 10th - 15:21
**baseline** (+/- 0ms)

malformed entry without heading
just some text

# macOS M1 (B7F2A9): May 10th - 16:04
**-10ms** (+/- 5ms)
"#;

    let entries = parse_run_entries(text);
    // Lines without a "# " heading that appear between entries get merged
    // into the preceding entry. Two actual heading boundaries = 2 entries.
    assert_eq!(entries.len(), 2);
    // First entry preserves the malformed text in its body
    let first_body = match &entries[0] {
        ParsedSummaryRunEntry::Parsed(e) => e.body.clone(),
        ParsedSummaryRunEntry::Raw(r) => r.clone(),
    };
    assert!(first_body.contains("malformed entry without heading"));
    // Second entry is the numeric one
    assert!(entries[1].to_markdown().contains("**-10ms** (+/- 5ms)"));
}

#[test]
fn test_append_numeric_after_no_change() {
    let mut runs = vec![ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 10th - 15:21".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    })];

    let new_entry = SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 10th - 16:04".to_string(),
        body: "**-10ms** (+/- 5ms)".to_string(),
        raw: String::new(),
    };

    // Build a comparison that represents Faster (numeric)
    let current = vec![benchmark_case("a", 90.0)];
    let previous = vec![benchmark_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));
    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Faster);

    append_or_replace_run_entry(&mut runs, new_entry, &comparison);

    assert_eq!(runs.len(), 2);
    assert!(runs[1].to_markdown().contains("**-10ms** (+/- 5ms)"));
}

#[test]
fn test_append_first_no_change() {
    let mut runs: Vec<ParsedSummaryRunEntry> = Vec::new();

    let new_entry = SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 10th - 15:21".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    };

    let current = vec![benchmark_case("a", 100.0)];
    let previous = vec![benchmark_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));
    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );

    append_or_replace_run_entry(&mut runs, new_entry, &comparison);

    assert_eq!(runs.len(), 1);
    assert!(
        runs[0]
            .to_markdown()
            .contains("no measurable change since last benchmark")
    );
}

#[test]
fn test_replace_consecutive_no_change_same_system() {
    let mut runs = vec![ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 11th - 12:40".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    })];

    let new_entry = SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 11th - 13:25".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    };

    let current = vec![benchmark_case("a", 100.0)];
    let previous = vec![benchmark_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    append_or_replace_run_entry(&mut runs, new_entry, &comparison);

    assert_eq!(runs.len(), 1);
    let markdown = runs[0].to_markdown();
    assert!(markdown.contains("May 11th - 13:25"));
    assert!(!markdown.contains("May 11th - 12:40"));
}

#[test]
fn test_case_set_changed_no_change_appends_instead_of_replacing() {
    let mut runs = vec![ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 11th - 12:40".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    })];

    let previous = vec![benchmark_case("a", 100.0)];
    let current = vec![benchmark_case("a", 100.0), benchmark_case("b", 20.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));
    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );
    assert!(comparison.case_set_changed);

    let new_entry = SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 11th - 13:25".to_string(),
        body: comparison.format_run_change_line(),
        raw: String::new(),
    };

    append_or_replace_run_entry(&mut runs, new_entry, &comparison);

    assert_eq!(runs.len(), 2);
    assert!(runs[0].to_markdown().contains("May 11th - 12:40"));
    assert!(runs[1].to_markdown().contains("case set changed"));
    assert!(runs[1].to_markdown().contains("May 11th - 13:25"));
}

#[test]
fn test_no_replace_for_different_system() {
    let mut runs = vec![ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "Linux x64".to_string(),
        public_system_id: "A91C".to_string(),
        timestamp_text: "May 11th - 12:40".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    })];

    let new_entry = SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 11th - 13:25".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    };

    let current = vec![benchmark_case("a", 100.0)];
    let previous = vec![benchmark_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    append_or_replace_run_entry(&mut runs, new_entry, &comparison);

    assert_eq!(runs.len(), 2);
    assert!(runs[0].to_markdown().contains("Linux x64"));
    assert!(runs[0].to_markdown().contains("May 11th - 12:40"));
    assert!(runs[1].to_markdown().contains("macOS M1"));
    assert!(runs[1].to_markdown().contains("May 11th - 13:25"));
}

#[test]
fn test_no_change_after_meaningful_change_appends() {
    let mut runs = vec![
        ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
            suite_kind_label: "End-to-end CLI".to_string(),
            display_name: "macOS M1".to_string(),
            public_system_id: "B7F2A9".to_string(),
            timestamp_text: "May 11th - 12:40".to_string(),
            body: "no measurable change since last benchmark".to_string(),
            raw: String::new(),
        }),
        ParsedSummaryRunEntry::Parsed(SummaryRunEntry {
            suite_kind_label: "End-to-end CLI".to_string(),
            display_name: "macOS M1".to_string(),
            public_system_id: "B7F2A9".to_string(),
            timestamp_text: "May 11th - 13:25".to_string(),
            body: "**-10ms** (+/- 2ms)".to_string(),
            raw: String::new(),
        }),
    ];

    let new_entry = SummaryRunEntry {
        suite_kind_label: "End-to-end CLI".to_string(),
        display_name: "macOS M1".to_string(),
        public_system_id: "B7F2A9".to_string(),
        timestamp_text: "May 11th - 14:00".to_string(),
        body: "no measurable change since last benchmark".to_string(),
        raw: String::new(),
    };

    let current = vec![benchmark_case("a", 100.0)];
    let previous = vec![benchmark_case("a", 100.0)];

    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    append_or_replace_run_entry(&mut runs, new_entry, &comparison);

    assert_eq!(runs.len(), 3);
    assert!(runs[0].to_markdown().contains("May 11th - 12:40"));
    assert!(runs[1].to_markdown().contains("May 11th - 13:25"));
    assert!(runs[2].to_markdown().contains("May 11th - 14:00"));
}

#[test]
fn test_public_id_from_system_heading() {
    assert_eq!(
        public_id_from_system_heading("## macOS Apple Silicon (6D851D)"),
        Some("6D851D")
    );
    assert_eq!(
        public_id_from_system_heading("## Linux x64 (A91C)"),
        Some("A91C")
    );
    assert_eq!(public_id_from_system_heading("# macOS M1 (B7F2A9)"), None);
    assert_eq!(public_id_from_system_heading("## macOS M1"), None);
    assert_eq!(public_id_from_system_heading("malformed"), None);
}

#[test]
fn test_parse_summary_blocks_exact_heading_match() {
    let text = r##"## macOS M1 (B7F2A9)
Change: -12ms
Initial: ~80ms

## Linux x64 (A91C)
Change: baseline
Initial: ~120ms

## Another macOS M1 (B7F2A9)
Change: 0ms
Initial: ~90ms
"##;

    let blocks = parse_summary_blocks(text, "B7F2A9", "End-to-end CLI");
    // Both blocks with heading ID "B7F2A9" are removed (current system).
    // Only "Linux x64 (A91C)" should remain.
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].contains("Linux x64 (A91C)"));
}

#[test]
fn test_parse_summary_blocks_preserves_block_with_id_in_body() {
    let text = r##"## Linux x64 (A91C)
Change: baseline
Note: compared against B7F2A9
Initial: ~120ms
"##;

    let blocks = parse_summary_blocks(text, "B7F2A9", "End-to-end CLI");
    // The ID "B7F2A9" appears only in the body text, not the heading.
    // The block should be preserved because matching is heading-only.
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].contains("Linux x64 (A91C)"));
}

#[test]
fn test_parse_summary_blocks_preserves_malformed_heading() {
    let text = r##"## Malformed heading no parens
Change: baseline
Initial: ~120ms

## Linux x64 (A91C)
Change: baseline
Initial: ~120ms
"##;

    let blocks = parse_summary_blocks(text, "B7F2A9", "End-to-end CLI");
    // Malformed heading without parseable ID should be preserved.
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].contains("Malformed heading no parens"));
    assert!(blocks[1].contains("Linux x64 (A91C)"));
}

#[test]
fn test_parse_summary_blocks_with_suite_kind_prefix() {
    let text = r##"## End-to-end CLI / macOS M1 (B7F2A9)
Change: -12ms
Initial: ~80ms

## Frontend phases / macOS M1 (B7F2A9)
Change: baseline
Initial: ~120ms

## Linux x64 (A91C)
Change: baseline
Initial: ~120ms
"##;

    // When updating End-to-end CLI for B7F2A9, only that block should be removed.
    let blocks = parse_summary_blocks(text, "B7F2A9", "End-to-end CLI");
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].contains("Frontend phases / macOS M1 (B7F2A9)"));
    assert!(blocks[1].contains("Linux x64 (A91C)"));
}

#[test]
fn test_monthly_summary_with_two_suite_kinds_keeps_blocks_separate() {
    // Simulate a summary that already has an End-to-end CLI block and run.
    let existing_content = r#"# May 2026 Summary

## End-to-end CLI / macOS M1 (B7F2A9)
Change since initial benchmark: baseline
Initial: all ~80ms
Latest: all ~80ms
Case spread latest: ~6ms

---------------------

# End-to-end CLI / macOS M1 (B7F2A9): May 10th - 15:21
**baseline**; 1 cases
Avg: all ~80ms, ungrouped ~80ms
"#;

    // Parse as if we're updating for Frontend phases on the same system.
    let (other_blocks, existing_runs) =
        parse_existing_summary(existing_content, "B7F2A9", "Frontend phases");

    // The existing CLI block should be preserved because it's a different suite kind.
    assert_eq!(other_blocks.len(), 1);
    assert!(other_blocks[0].contains("End-to-end CLI / macOS M1 (B7F2A9)"));

    // The existing CLI run entry should also be preserved.
    assert_eq!(existing_runs.len(), 1);
    assert!(
        existing_runs[0]
            .to_markdown()
            .contains("End-to-end CLI / macOS M1 (B7F2A9)")
    );
}

#[test]
fn test_update_monthly_summary_fixed_thread_run_is_noop() {
    // A fixed-thread run must no-op before any summary read or write so the
    // tracked summary stays a default-thread signal. The early return means
    // no runs.jsonl read and no summary file access occurs.
    let cases = vec![benchmark_case("check_docs", 80.0)];
    let mut run = benchmark_run(cases);
    run.thread_count = Some(4);

    let comparison = BenchmarkComparison::new(&run.cases, None);

    let result = update_monthly_summary(&run, &comparison);
    assert!(result.is_ok(), "fixed-thread run should no-op cleanly");
}
