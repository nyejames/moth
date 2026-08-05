use super::*;

#[test]
fn run_policy_rejects_zero_measured_iterations() {
    let error = BenchmarkRunPolicy::new(0, BenchmarkSelection::Full, BenchmarkRecording::ReadOnly)
        .expect_err("zero measured iterations must be impossible");

    assert_eq!(error, BenchmarkRunPolicyError::ZeroMeasuredIterations);
}

#[test]
fn run_policy_keeps_selection_and_recording_explicit() {
    let full_record =
        BenchmarkRunPolicy::new(10, BenchmarkSelection::Full, BenchmarkRecording::Record)
            .expect("full recording policy should be valid");
    let quick_read_only =
        BenchmarkRunPolicy::new(3, BenchmarkSelection::Quick, BenchmarkRecording::ReadOnly)
            .expect("quick read-only policy should be valid");

    assert_eq!(full_record.measured_iterations().get(), 10);
    assert_eq!(full_record.selection(), BenchmarkSelection::Full);
    assert_eq!(full_record.recording(), BenchmarkRecording::Record);
    assert!(full_record.selects_case(false));

    assert_eq!(quick_read_only.measured_iterations().get(), 3);
    assert_eq!(quick_read_only.selection(), BenchmarkSelection::Quick);
    assert_eq!(quick_read_only.recording(), BenchmarkRecording::ReadOnly);
    assert!(quick_read_only.selects_case(true));
    assert!(!quick_read_only.selects_case(false));
}

#[test]
fn run_policy_rejects_quick_recording() {
    let error = BenchmarkRunPolicy::new(3, BenchmarkSelection::Quick, BenchmarkRecording::Record)
        .expect_err("recording paths must remain full-suite only");

    assert_eq!(
        error,
        BenchmarkRunPolicyError::RecordingRequiresFullSelection
    );
}

fn make_case(name: &str, mean_ms: f64) -> BenchmarkCaseResult {
    make_grouped_case(name, "ungrouped", mean_ms)
}

fn make_grouped_case(name: &str, group_name: &str, mean_ms: f64) -> BenchmarkCaseResult {
    make_grouped_case_with_stddev(name, group_name, mean_ms, 0.0)
}

fn make_grouped_case_with_stddev(
    name: &str,
    group_name: &str,
    mean_ms: f64,
    stddev_ms: f64,
) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        case_id: name.to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: format!("{name}_workload"),
            source_fingerprint: format!("{name}_source_fp"),
            measurement_fingerprint: format!("{name}_measurement_fp"),
        }),
        group_name: group_name.to_string(),
        runner: BenchmarkRunner::Cli {
            command: crate::benchmark_manifest::CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
        mean_ms,
        median_ms: mean_ms,
        stddev_ms,
        observations: BenchmarkCaseObservations::default(),
    }
}

fn make_case_with_observations(
    name: &str,
    mean_ms: f64,
    observations: BenchmarkCaseObservations,
) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        case_id: name.to_string(),
        identity: Some(BenchmarkMeasurementIdentity {
            workload_id: format!("{name}_workload"),
            source_fingerprint: format!("{name}_source_fp"),
            measurement_fingerprint: format!("{name}_measurement_fp"),
        }),
        group_name: "ungrouped".to_string(),
        runner: BenchmarkRunner::Cli {
            command: crate::benchmark_manifest::CliBenchmarkCommand::Check,
            args: Vec::new(),
        },
        mean_ms,
        median_ms: mean_ms,
        stddev_ms: 0.0,
        observations,
    }
}

fn observations_with_stages(stages: &[(&str, f64)]) -> BenchmarkCaseObservations {
    BenchmarkCaseObservations {
        stage_timings: stages
            .iter()
            .map(|(name, value)| BenchmarkMetric {
                name: name.to_string(),
                value: *value,
            })
            .collect(),
        counters: Vec::new(),
    }
}

#[test]
fn test_calculate_mean() {
    assert_eq!(calculate_mean(&[100.0, 200.0, 300.0]), 200.0);
    assert_eq!(calculate_mean(&[]), 0.0);
    assert_eq!(calculate_mean(&[42.0]), 42.0);
}

#[test]
fn test_calculate_stddev() {
    let values = vec![100.0, 200.0, 300.0];
    let mean = calculate_mean(&values);
    let stddev = calculate_stddev(&values, mean);
    assert!((stddev - 81.65).abs() < 0.1);

    let values = vec![100.0];
    let mean = calculate_mean(&values);
    let stddev = calculate_stddev(&values, mean);
    assert_eq!(stddev, 0.0);
}

#[test]
fn test_calculate_median() {
    assert_eq!(calculate_median(&[]), 0.0);
    assert_eq!(calculate_median(&[42.0]), 42.0);
    assert_eq!(calculate_median(&[30.0, 10.0, 20.0]), 20.0);
    assert_eq!(calculate_median(&[40.0, 10.0, 30.0, 20.0]), 25.0);
}

#[test]
fn test_suite_stats_from_case_results() {
    let cases = vec![make_case("a", 100.0), make_case("b", 200.0)];
    let stats = SuiteStats::from_case_results(&cases);
    assert_eq!(stats.average_ms, 150.0);
    assert!(stats.case_spread_ms > 0.0);
}

#[test]
fn test_calculate_group_stats() {
    let cases = vec![
        make_grouped_case("speed-check", "core", 100.0),
        make_grouped_case("speed-build", "core", 120.0),
        make_grouped_case("docs", "docs", 80.0),
    ];

    let stats = calculate_group_stats(&cases);

    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].group_name, "core");
    assert_eq!(stats[0].case_count, 2);
    assert_eq!(stats[0].average_ms, 110.0);
    assert_eq!(stats[1].group_name, "docs");
    assert_eq!(stats[1].case_count, 1);
    assert_eq!(stats[1].average_ms, 80.0);
}

#[test]
fn test_calculate_group_stats_uses_stable_group_order() {
    let cases = vec![
        make_grouped_case("custom-z", "zeta", 10.0),
        make_grouped_case("borrow-case", "borrow", 20.0),
        make_grouped_case("docs-case", "docs", 30.0),
        make_grouped_case("custom-a", "alpha", 40.0),
        make_grouped_case("stress-case", "stress", 50.0),
        make_grouped_case("module-case", "module", 60.0),
        make_grouped_case("core-case", "core", 70.0),
    ];

    let group_names: Vec<String> = calculate_group_stats(&cases)
        .into_iter()
        .map(|stats| stats.group_name)
        .collect();

    assert_eq!(
        group_names,
        vec![
            "core", "docs", "stress", "module", "borrow", "alpha", "zeta"
        ]
    );
}

#[test]
fn test_comparison_first_run_baseline() {
    let current = vec![make_case("a", 100.0), make_case("b", 200.0)];
    let comparison = BenchmarkComparison::new(&current, None);

    assert!(comparison.overall_mean_delta_ms.is_none());
    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Baseline);
    assert_eq!(comparison.current_case_count, 2);
    assert_eq!(comparison.previous_case_count, 0);
    assert_eq!(comparison.compared_case_count, 0);
    assert!(!comparison.case_set_changed);
    assert_eq!(
        comparison.format_run_change_line(),
        "**baseline**; 2 cases, avg ~150ms"
    );
}

#[test]
fn test_comparison_no_overlapping_cases_becomes_baseline_with_case_set_changed() {
    let current = vec![make_case("x", 100.0)];
    let previous = vec![make_case("y", 200.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert!(comparison.overall_mean_delta_ms.is_none());
    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Baseline);
    assert!(comparison.case_set_changed);
    assert_eq!(
        comparison.format_run_change_line(),
        "case set changed: no shared cases; 1 current, 1 previous"
    );
}

#[test]
fn test_comparison_pure_slower_cases() {
    let current = vec![make_case("a", 120.0), make_case("b", 220.0)];
    let previous = vec![make_case("a", 100.0), make_case("b", 200.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.overall_mean_delta_ms, Some(20.0));
    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Slower);
    assert_eq!(comparison.slower_case_count, 2);
    assert_eq!(comparison.faster_case_count, 0);
    assert_eq!(
        comparison.format_run_change_line(),
        "**+20ms avg**; 0 faster, 2 slower; 2/2 cases"
    );
}

#[test]
fn stable_id_and_equal_fingerprint_are_comparable() {
    let current = vec![make_case("stable_case", 120.0)];
    let previous = vec![make_case("stable_case", 100.0)];

    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.compared_case_count, 1);
    assert_eq!(comparison.workload_changed_case_count, 0);
    assert_eq!(comparison.cases[0].case_id, "stable_case");
    assert_eq!(comparison.overall_mean_delta_ms, Some(20.0));
}

#[test]
fn changed_fingerprint_is_excluded_and_reported_without_speed_delta() {
    let mut current_first = make_case("first", 80.0);
    current_first.identity.as_mut().unwrap().source_fingerprint = "first_changed".to_string();
    let mut current_second = make_case("second", 250.0);
    current_second.identity.as_mut().unwrap().source_fingerprint = "second_changed".to_string();
    let current = vec![current_first, current_second];
    let previous = vec![make_case("first", 100.0), make_case("second", 200.0)];

    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.compared_case_count, 0);
    assert_eq!(comparison.overall_mean_delta_ms, None);
    assert!(!comparison.case_set_changed);
    assert_eq!(
        comparison.workload_changed_case_ids,
        ["first".to_string(), "second".to_string()]
    );
    assert_eq!(
        comparison.format_run_change_line(),
        "no comparable unchanged workloads; workload changed: 2 cases (first, second)"
    );
}

#[test]
fn timing_delta_uses_only_comparable_unchanged_workloads() {
    let comparable = make_case("comparable", 120.0);
    let mut changed = make_case("changed", 10.0);
    changed.identity.as_mut().unwrap().source_fingerprint = "changed_fingerprint_v2".to_string();
    let current = vec![comparable, changed];
    let previous = vec![
        make_case("comparable", 100.0),
        make_case("changed", 1_000.0),
    ];

    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.compared_case_count, 1);
    assert_eq!(comparison.overall_mean_delta_ms, Some(20.0));
    assert_eq!(comparison.cases[0].case_id, "comparable");
    assert_eq!(comparison.workload_changed_case_ids, ["changed"]);
    assert_eq!(
        comparison.format_run_change_line(),
        "**+20ms avg**; 0 faster, 1 slower; 1/2 cases; workload changed: 1 case (changed)"
    );
}

#[test]
fn renamed_runner_path_with_stable_id_and_fingerprint_remains_comparable() {
    let mut current = make_case("stable_case", 90.0);
    current.runner = BenchmarkRunner::Cli {
        command: crate::benchmark_manifest::CliBenchmarkCommand::Check,
        args: vec!["benchmarks/renamed/speed-test.moth".to_string()],
    };
    let mut previous = make_case("stable_case", 100.0);
    previous.runner = BenchmarkRunner::Cli {
        command: crate::benchmark_manifest::CliBenchmarkCommand::Check,
        args: vec!["benchmarks/speed-test.moth".to_string()],
    };

    let comparison = BenchmarkComparison::new(&[current], Some(&[previous]));

    assert_eq!(comparison.compared_case_count, 1);
    assert_eq!(comparison.workload_changed_case_count, 0);
    assert_eq!(comparison.overall_mean_delta_ms, Some(-10.0));
}

#[test]
fn quick_subset_filters_intentional_previous_cases() {
    let current = vec![make_case("quick_a", 90.0), make_case("quick_c", 290.0)];
    let previous = vec![
        make_case("quick_a", 100.0),
        make_case("full_only_b", 200.0),
        make_case("quick_c", 300.0),
    ];

    let comparison = BenchmarkComparison::for_quick_subset(&current, Some(&previous));

    assert!(!comparison.case_set_changed);
    assert_eq!(comparison.previous_case_count, 2);
    assert_eq!(comparison.compared_case_count, 2);
    assert_eq!(comparison.faster_case_count, 2);
}

#[test]
fn case_set_and_workload_changes_are_reported_separately() {
    let mut changed = make_case("shared", 80.0);
    changed.identity.as_mut().unwrap().source_fingerprint = "changed_fingerprint".to_string();
    let current = vec![changed, make_case("added", 50.0)];
    let previous = vec![make_case("shared", 100.0), make_case("removed", 60.0)];

    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert!(comparison.case_set_changed);
    assert_eq!(comparison.workload_changed_case_ids, ["shared"]);
    assert_eq!(comparison.overall_mean_delta_ms, None);
    assert_eq!(
        comparison.format_run_change_line(),
        "case set changed: no comparable unchanged workloads; 2 current, 2 previous; workload changed: 1 case (shared)"
    );
}

#[test]
fn test_comparison_case_set_changed_when_current_has_new_case() {
    let current = vec![
        make_case("a", 120.0),
        make_case("b", 220.0),
        make_case("c", 300.0),
    ];
    let previous = vec![make_case("a", 100.0), make_case("b", 200.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.overall_mean_delta_ms, Some(20.0));
    assert!(comparison.case_set_changed);
    assert_eq!(comparison.compared_case_count, 2);
    assert_eq!(
        comparison.format_run_change_line(),
        "case set changed: avg +20ms on 2/3 shared cases; 2 slower, 0 faster"
    );
}

#[test]
fn test_comparison_case_set_changed_when_previous_had_removed_case() {
    let current = vec![make_case("a", 120.0)];
    let previous = vec![make_case("a", 100.0), make_case("b", 200.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.overall_mean_delta_ms, Some(20.0));
    assert!(comparison.case_set_changed);
    assert_eq!(comparison.compared_case_count, 1);
    assert_eq!(
        comparison.format_run_change_line(),
        "case set changed: avg +20ms on 1/2 shared cases; 1 slower, 0 faster"
    );
}

#[test]
fn test_comparison_pure_faster_cases() {
    let current = vec![make_case("a", 80.0)];
    let previous = vec![make_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.overall_mean_delta_ms, Some(-20.0));
    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Faster);
    assert_eq!(comparison.faster_case_count, 1);
    assert_eq!(
        comparison.format_run_change_line(),
        "**-20ms avg**; 1 faster, 0 slower; 1/1 cases"
    );
}

#[test]
fn test_comparison_zero_delta_is_no_measurable_change() {
    let current = vec![make_case("a", 100.0)];
    let previous = vec![make_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.overall_mean_delta_ms, Some(0.0));
    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );
    assert_eq!(comparison.unchanged_case_count, 1);
    assert_eq!(
        comparison.format_run_change_line(),
        "no measurable change: avg 0ms; 1/1 cases"
    );
}

#[test]
fn test_comparison_delta_within_case_threshold_is_no_change() {
    let current = vec![make_grouped_case_with_stddev("a", "core", 103.0, 2.0)];
    let previous = vec![make_grouped_case_with_stddev("a", "core", 100.0, 2.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );

    let combined_stddev = (8.0_f64).sqrt();
    let expected_threshold = 2.0_f64.max(100.0 * 0.03).max(combined_stddev * 2.0);
    assert!((comparison.cases[0].threshold_ms - expected_threshold).abs() < 0.001);
}

#[test]
fn test_comparison_mixed_when_cases_move_opposite_directions() {
    let current = vec![make_case("a", 90.0), make_case("b", 210.0)];
    let previous = vec![make_case("a", 100.0), make_case("b", 200.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.overall_mean_delta_ms, Some(0.0));
    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Mixed);
    assert_eq!(comparison.faster_case_count, 1);
    assert_eq!(comparison.slower_case_count, 1);
    assert_eq!(
        comparison.format_run_change_line(),
        "mixed: avg 0ms; 1 faster, 1 slower; 2/2 cases"
    );
}

#[test]
fn test_comparison_single_large_regression_is_not_hidden_by_unchanged_cases() {
    let current = vec![
        make_case("a", 100.0),
        make_case("b", 100.0),
        make_case("c", 130.0),
    ];
    let previous = vec![
        make_case("a", 100.0),
        make_case("b", 100.0),
        make_case("c", 100.0),
    ];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.change_kind, BenchmarkChangeKind::Slower);
    assert_eq!(comparison.slower_case_count, 1);
    assert_eq!(comparison.unchanged_case_count, 2);
    assert_eq!(
        comparison.format_run_change_line(),
        "**+10ms avg**; 0 faster, 1 slower; 3/3 cases"
    );
}

#[test]
fn test_comparison_small_delta_with_zero_stddev_is_no_change() {
    let current = vec![make_case("a", 99.3)];
    let previous = vec![make_case("a", 100.0)];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert!(comparison.overall_mean_delta_ms.unwrap().abs() < 1.0);
    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );
}

#[test]
fn test_comparison_group_counts_aggregate_by_group() {
    let current = vec![
        make_grouped_case("core-fast", "core", 90.0),
        make_grouped_case("core-slow", "core", 120.0),
        make_grouped_case("docs-same", "docs", 50.0),
    ];
    let previous = vec![
        make_grouped_case("core-fast", "core", 100.0),
        make_grouped_case("core-slow", "core", 100.0),
        make_grouped_case("docs-same", "docs", 50.0),
    ];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    assert_eq!(comparison.groups.len(), 2);
    assert_eq!(comparison.groups[0].group_name, "core");
    assert_eq!(comparison.groups[0].faster_count, 1);
    assert_eq!(comparison.groups[0].slower_count, 1);
    assert_eq!(comparison.groups[0].unchanged_count, 0);
    assert_eq!(comparison.groups[0].previous_average_ms, Some(100.0));
    assert_eq!(comparison.groups[0].current_average_ms, 105.0);
    assert_eq!(comparison.groups[0].delta_ms, Some(5.0));

    assert_eq!(comparison.groups[1].group_name, "docs");
    assert_eq!(comparison.groups[1].unchanged_count, 1);
}

#[test]
fn test_compare_observations_overlapping_stages_only() {
    let current = observations_with_stages(&[("ast_ms", 55.0), ("hir_ms", 12.0)]);
    let previous = observations_with_stages(&[("ast_ms", 50.0), ("borrow_ms", 8.0)]);

    let comparison = compare_observations(&current, &previous, &BenchmarkThresholds::DEFAULT);

    assert_eq!(comparison.stage_comparisons.len(), 1);
    assert_eq!(comparison.stage_comparisons[0].stage_name, "ast_ms");
    assert_eq!(comparison.stage_comparisons[0].delta_ms, 5.0);
}

#[test]
fn test_compare_observations_sorted_by_abs_delta() {
    let current = observations_with_stages(&[("ast_ms", 55.0), ("hir_ms", 20.0)]);
    let previous = observations_with_stages(&[("ast_ms", 50.0), ("hir_ms", 10.0)]);

    let comparison = compare_observations(&current, &previous, &BenchmarkThresholds::DEFAULT);

    assert_eq!(comparison.stage_comparisons.len(), 2);
    assert_eq!(comparison.stage_comparisons[0].stage_name, "hir_ms");
    assert_eq!(comparison.stage_comparisons[1].stage_name, "ast_ms");
}

#[test]
fn test_compare_observations_empty_when_no_overlap() {
    let current = observations_with_stages(&[("ast_ms", 55.0)]);
    let previous = observations_with_stages(&[("hir_ms", 10.0)]);

    let comparison = compare_observations(&current, &previous, &BenchmarkThresholds::DEFAULT);

    assert!(comparison.stage_comparisons.is_empty());
}

#[test]
fn test_stage_threshold_uses_five_percent_or_floor() {
    let thresholds = BenchmarkThresholds {
        minimum_stage_delta_ms: 0.5,
        minimum_stage_delta_ratio: 0.05,
        ..BenchmarkThresholds::DEFAULT
    };

    // 5% of 100 ms = 5 ms, which is > 0.5 ms
    assert!((stage_threshold_ms(100.0, 100.0, &thresholds) - 5.0).abs() < 0.001);
    // 5% of 5 ms = 0.25 ms, which is < 0.5 ms, so floor applies
    assert!((stage_threshold_ms(5.0, 5.0, &thresholds) - 0.5).abs() < 0.001);
}

#[test]
fn test_classify_stage_change() {
    assert_eq!(
        classify_stage_change(-6.0, 5.0),
        BenchmarkCaseChangeKind::Faster
    );
    assert_eq!(
        classify_stage_change(6.0, 5.0),
        BenchmarkCaseChangeKind::Slower
    );
    assert_eq!(
        classify_stage_change(0.0, 5.0),
        BenchmarkCaseChangeKind::NoMeasurableChange
    );
    assert_eq!(
        classify_stage_change(4.0, 5.0),
        BenchmarkCaseChangeKind::NoMeasurableChange
    );
}

#[test]
fn test_case_threshold_uses_max_of_absolute_ratio_and_stddev() {
    let thresholds = BenchmarkThresholds {
        minimum_case_delta_ms: 2.0,
        minimum_case_delta_ratio: 0.03,
        stddev_multiplier: 2.0,
        minimum_stage_delta_ms: 1.0,
        minimum_stage_delta_ratio: 0.05,
    };

    let current = make_grouped_case_with_stddev("a", "core", 103.0, 2.0);
    let previous = make_grouped_case_with_stddev("a", "core", 100.0, 2.0);

    let threshold = case_threshold_ms(&current, &previous, &thresholds);
    let combined_stddev = (8.0_f64).sqrt();
    let expected = 2.0_f64.max(100.0 * 0.03).max(combined_stddev * 2.0);

    assert!((threshold - expected).abs() < 0.001);
}

#[test]
fn test_comparison_small_fast_case_uses_absolute_floor() {
    let thresholds = BenchmarkThresholds {
        minimum_case_delta_ms: 2.0,
        minimum_case_delta_ratio: 0.03,
        stddev_multiplier: 2.0,
        ..BenchmarkThresholds::DEFAULT
    };
    let current = vec![make_case("fast", 11.9)];
    let previous = vec![make_case("fast", 10.0)];

    let comparison =
        BenchmarkComparison::new_with_thresholds(&current, Some(&previous), &thresholds);

    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );
    assert!((comparison.cases[0].threshold_ms - 2.0).abs() < 0.001);
}

#[test]
fn test_comparison_large_slow_case_uses_ratio_floor() {
    let thresholds = BenchmarkThresholds {
        minimum_case_delta_ms: 2.0,
        minimum_case_delta_ratio: 0.03,
        stddev_multiplier: 2.0,
        ..BenchmarkThresholds::DEFAULT
    };
    let current = vec![make_case("slow", 1025.0)];
    let previous = vec![make_case("slow", 1000.0)];

    let comparison =
        BenchmarkComparison::new_with_thresholds(&current, Some(&previous), &thresholds);

    assert_eq!(
        comparison.change_kind,
        BenchmarkChangeKind::NoMeasurableChange
    );
    assert!((comparison.cases[0].threshold_ms - 30.0).abs() < 0.001);
}

#[test]
fn test_calculate_stage_movement_sums_deltas_and_counts() {
    let current = vec![
        make_case_with_observations("a", 100.0, observations_with_stages(&[("ast_ms", 55.0)])),
        make_case_with_observations("b", 100.0, observations_with_stages(&[("ast_ms", 45.0)])),
    ];
    let previous = vec![
        make_case_with_observations("a", 100.0, observations_with_stages(&[("ast_ms", 50.0)])),
        make_case_with_observations("b", 100.0, observations_with_stages(&[("ast_ms", 50.0)])),
    ];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    let movements = calculate_stage_movement(&comparison);
    assert_eq!(movements.len(), 1);
    assert_eq!(movements[0].stage_name, "ast_ms");
    assert_eq!(movements[0].total_delta_ms, 0.0);
    assert_eq!(movements[0].case_count, 2);
    assert_eq!(movements[0].faster_count, 1);
    assert_eq!(movements[0].slower_count, 1);
}

#[test]
fn test_calculate_stage_movement_sorts_by_abs_delta() {
    let current = vec![make_case_with_observations(
        "a",
        100.0,
        observations_with_stages(&[("ast_ms", 55.0), ("hir_ms", 20.0)]),
    )];
    let previous = vec![make_case_with_observations(
        "a",
        100.0,
        observations_with_stages(&[("ast_ms", 50.0), ("hir_ms", 10.0)]),
    )];
    let comparison = BenchmarkComparison::new(&current, Some(&previous));

    let movements = calculate_stage_movement(&comparison);
    assert_eq!(movements.len(), 2);
    assert_eq!(movements[0].stage_name, "hir_ms");
    assert_eq!(movements[1].stage_name, "ast_ms");
}

#[test]
fn test_format_stage_movement_line_limits_to_top_three() {
    let movements = vec![
        BenchmarkStageMovement {
            stage_name: "ast_ms".to_string(),
            total_delta_ms: -5.0,
            case_count: 2,
            faster_count: 2,
            slower_count: 0,
        },
        BenchmarkStageMovement {
            stage_name: "hir_ms".to_string(),
            total_delta_ms: -3.0,
            case_count: 2,
            faster_count: 2,
            slower_count: 0,
        },
        BenchmarkStageMovement {
            stage_name: "headers_ms".to_string(),
            total_delta_ms: -2.0,
            case_count: 2,
            faster_count: 2,
            slower_count: 0,
        },
        BenchmarkStageMovement {
            stage_name: "borrow_ms".to_string(),
            total_delta_ms: -1.0,
            case_count: 2,
            faster_count: 2,
            slower_count: 0,
        },
    ];

    let line = format_stage_movement_line(&movements, &BenchmarkThresholds::DEFAULT).unwrap();
    assert!(line.contains("ast"));
    assert!(line.contains("hir"));
    assert!(line.contains("headers"));
    assert!(!line.contains("borrow"));
}

#[test]
fn test_format_stage_movement_line_hides_below_threshold() {
    let thresholds = BenchmarkThresholds {
        minimum_stage_delta_ms: 1.0,
        ..BenchmarkThresholds::DEFAULT
    };
    let movements = vec![BenchmarkStageMovement {
        stage_name: "ast_ms".to_string(),
        total_delta_ms: -0.5,
        case_count: 1,
        faster_count: 1,
        slower_count: 0,
    }];

    assert!(format_stage_movement_line(&movements, &thresholds).is_none());
}

#[test]
fn test_format_stage_movement_line_uses_friendly_labels() {
    let movements = vec![BenchmarkStageMovement {
        stage_name: "ast_build_environment_ms".to_string(),
        total_delta_ms: -3.0,
        case_count: 1,
        faster_count: 1,
        slower_count: 0,
    }];

    let line = format_stage_movement_line(&movements, &BenchmarkThresholds::DEFAULT).unwrap();
    assert!(line.contains("ast env"));
    assert!(!line.contains("ast_build_environment_ms"));
}

#[test]
fn test_format_stage_movement_line_none_when_empty() {
    assert!(format_stage_movement_line(&[], &BenchmarkThresholds::DEFAULT).is_none());
}

#[test]
fn test_friendly_stage_label_maps_known_names() {
    assert_eq!(friendly_stage_label("tokenize_ms"), "tokenize");
    assert_eq!(friendly_stage_label("headers_ms"), "headers");
    assert_eq!(friendly_stage_label("file_prepare_ms"), "file prep");
    assert_eq!(friendly_stage_label("dependency_sort_ms"), "sort");
    assert_eq!(friendly_stage_label("ast_ms"), "ast");
    assert_eq!(friendly_stage_label("ast_build_environment_ms"), "ast env");
    assert_eq!(friendly_stage_label("ast_emit_nodes_ms"), "ast emit");
    assert_eq!(friendly_stage_label("ast_finalize_ms"), "ast finalize");
    assert_eq!(
        friendly_stage_label("ast_function_body_parse_ms"),
        "ast func bodies"
    );
    assert_eq!(
        friendly_stage_label("ast_start_body_parse_ms"),
        "ast start body"
    );
    assert_eq!(
        friendly_stage_label("ast_const_template_parse_ms"),
        "ast const parse"
    );
    assert_eq!(
        friendly_stage_label("ast_const_template_fold_ms"),
        "ast const fold"
    );
    assert_eq!(friendly_stage_label("hir_ms"), "hir");
    assert_eq!(friendly_stage_label("borrow_ms"), "borrow");
    assert_eq!(
        friendly_stage_label("command.check.compile_project_frontend"),
        "check frontend"
    );
    assert_eq!(
        friendly_stage_label("stage0.module_root_discovery.total"),
        "module roots"
    );
    assert_eq!(friendly_stage_label("backend.js.lower_hir"), "js lower");
    assert_eq!(
        friendly_stage_label("backend.js.lower_linked_hir"),
        "js linked lower"
    );
    assert_eq!(friendly_stage_label("unknown_ms"), "unknown_ms");
}

#[test]
fn test_format_top_current_stages_shows_top_three() {
    let cases = vec![make_case_with_observations(
        "a",
        100.0,
        observations_with_stages(&[
            ("ast_ms", 80.0),
            ("hir_ms", 20.0),
            ("headers_ms", 10.0),
            ("borrow_ms", 5.0),
        ]),
    )];

    let line = format_top_current_stages(&cases).unwrap();
    assert!(line.starts_with("Top stages:"));
    assert!(line.contains("ast ~80ms"));
    assert!(line.contains("hir ~20ms"));
    assert!(line.contains("headers ~10ms"));
    assert!(!line.contains("borrow"));
}

#[test]
fn test_format_top_current_stages_averages_across_cases() {
    let cases = vec![
        make_case_with_observations("a", 100.0, observations_with_stages(&[("ast_ms", 80.0)])),
        make_case_with_observations("b", 100.0, observations_with_stages(&[("ast_ms", 100.0)])),
    ];

    let line = format_top_current_stages(&cases).unwrap();
    assert!(line.contains("ast ~90ms"));
}

#[test]
fn test_format_top_current_stages_none_when_empty() {
    let cases = vec![make_case("a", 100.0)];
    assert!(format_top_current_stages(&cases).is_none());
}
