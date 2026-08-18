use super::*;

#[test]
fn parses_exact_live_check_and_build_metrics() {
    for (command, required_name) in [
        (CliBenchmarkCommand::Check, "command.check.total"),
        (CliBenchmarkCommand::Build, "command.build.total"),
    ] {
        let stdout = format!(
            "MOTH_BENCH timing-schema 2\nMOTH_BENCH timing {required_name}=8ms\nMOTH_BENCH timing frontend.ast.total=2.5ms\n"
        );

        let observations = parse_stdout_observations(&stdout, command)
            .expect("exact live timing records should parse");

        assert_metric_value(&observations.stage_timings, required_name, 8.0);
        assert_metric_value(&observations.stage_timings, "frontend.ast.total", 2.5);
    }
}

#[test]
fn malformed_stable_timing_lines_fail_closed() {
    for line in [
        "MOTH_BENCH timing",
        "MOTH_BENCH timingcommand.check.total=1ms",
        "MOTH_BENCH timing =1ms",
        "MOTH_BENCH timing command.check.total=1",
        "MOTH_BENCH timing command.check.total=1ms trailing",
        "MOTH_BENCH timing command.check.total=1ms=2ms",
    ] {
        let error = parse_stdout_observations(line, CliBenchmarkCommand::Check)
            .expect_err("malformed stable timing records must fail");

        assert!(
            matches!(
                error,
                BenchmarkObservationError::MalformedTimingLine { .. }
                    | BenchmarkObservationError::InvalidMetricName { .. }
            ),
            "unexpected error for {line:?}: {error}"
        );
    }
}

#[test]
fn live_timing_names_and_order_are_registry_closed() {
    let unknown = parse_stdout_observations(
        concat!(
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing command.check.total=8ms\n",
            "MOTH_BENCH timing frontend.ast=2ms\n",
        ),
        CliBenchmarkCommand::Check,
    )
    .expect_err("provisional timing names must be rejected");
    assert_eq!(
        unknown,
        BenchmarkObservationError::UnknownTimingMetric {
            metric_name: "frontend.ast".to_owned()
        }
    );

    let reversed = parse_stdout_observations(
        concat!(
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing frontend.ast.total=2ms\n",
            "MOTH_BENCH timing command.check.total=8ms\n",
        ),
        CliBenchmarkCommand::Check,
    )
    .expect_err("timing rows must follow schema order");
    assert_eq!(
        reversed,
        BenchmarkObservationError::TimingMetricOutOfOrder {
            previous_metric_name: "frontend.ast.total".to_owned(),
            metric_name: "command.check.total".to_owned(),
        }
    );

    let sparse = parse_stdout_observations(
        concat!(
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing command.check.total=8ms\n",
            "MOTH_BENCH timing frontend.prepare=2ms\n",
            "MOTH_BENCH timing frontend.ast.total=3ms\n",
        ),
        CliBenchmarkCommand::Check,
    )
    .expect("valid sparse schema order should parse");
    assert_eq!(
        sparse
            .stage_timings
            .iter()
            .map(|metric| metric.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "command.check.total",
            "frontend.prepare",
            "frontend.ast.total"
        ]
    );
}

#[test]
fn timing_schema_header_is_required_once_and_current() {
    let missing = parse_stdout_observations(
        "MOTH_BENCH timing command.check.total=1ms",
        CliBenchmarkCommand::Check,
    )
    .expect_err("live timing observations require a schema header");
    assert_eq!(missing, BenchmarkObservationError::MissingTimingSchema);

    let duplicate = parse_stdout_observations(
        concat!(
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing command.check.total=1ms"
        ),
        CliBenchmarkCommand::Check,
    )
    .expect_err("duplicate schema headers must fail");
    assert_eq!(duplicate, BenchmarkObservationError::DuplicateTimingSchema);

    let future = parse_stdout_observations(
        "MOTH_BENCH timing-schema 3\nMOTH_BENCH timing command.check.total=1ms",
        CliBenchmarkCommand::Check,
    )
    .expect_err("future schema headers must fail closed");
    assert_eq!(
        future,
        BenchmarkObservationError::UnsupportedTimingSchema { version: 3 }
    );
}

#[test]
fn final_timing_records_must_not_repeat_a_metric() {
    let error = parse_stdout_observations(
        concat!(
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing command.check.total=1ms\n",
            "MOTH_BENCH timing command.check.total=2ms"
        ),
        CliBenchmarkCommand::Check,
    )
    .expect_err("final aggregate timing records must be unique");

    assert_eq!(
        error,
        BenchmarkObservationError::DuplicateTimingMetric {
            metric_name: "command.check.total".to_owned()
        }
    );
}

#[test]
fn non_finite_and_negative_values_fail() {
    for value in ["NaN", "inf", "-1"] {
        let timing_stdout =
            format!("MOTH_BENCH timing-schema 2\nMOTH_BENCH timing command.check.total={value}ms");
        let timing_error = parse_stdout_observations(&timing_stdout, CliBenchmarkCommand::Check)
            .expect_err("invalid timing values must fail");
        assert!(matches!(
            timing_error,
            BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "timing",
                ..
            }
        ));

        let counter_stdout = format!(
            "MOTH_BENCH timing-schema 2\nMOTH_BENCH timing command.check.total=1ms\nMOTH_BENCH counter work={value}"
        );
        let counter_error = parse_stdout_observations(&counter_stdout, CliBenchmarkCommand::Check)
            .expect_err("invalid counter values must fail");
        assert!(matches!(
            counter_error,
            BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "counter",
                ..
            }
        ));
    }
}

#[test]
fn required_cli_total_metric_must_match_command() {
    let check_error = parse_stdout_observations(
        "MOTH_BENCH timing-schema 2\nMOTH_BENCH timing frontend.ast.total=1ms",
        CliBenchmarkCommand::Check,
    )
    .expect_err("check total is required");
    assert_eq!(
        check_error,
        BenchmarkObservationError::MissingRequiredTiming {
            metric_name: "command.check.total"
        }
    );

    let build_error = parse_stdout_observations(
        "MOTH_BENCH timing-schema 2\nMOTH_BENCH timing command.check.total=1ms",
        CliBenchmarkCommand::Build,
    )
    .expect_err("build total is required");
    assert_eq!(
        build_error,
        BenchmarkObservationError::MissingRequiredTiming {
            metric_name: "command.build.total"
        }
    );
}

#[test]
fn repeated_final_timing_names_are_rejected() {
    let stdout = concat!(
        "MOTH_BENCH timing-schema 2\n",
        "MOTH_BENCH timing frontend.ast.total=2ms\n",
        "MOTH_BENCH timing frontend.ast.total=3ms\n",
        "MOTH_BENCH timing command.check.total=8ms\n",
    );

    let error = parse_stdout_observations(stdout, CliBenchmarkCommand::Check)
        .expect_err("repeated final aggregate metrics must be rejected");

    assert_eq!(
        error,
        BenchmarkObservationError::DuplicateTimingMetric {
            metric_name: "frontend.ast.total".to_owned()
        }
    );
}

#[test]
fn stable_counters_parse_and_sum_when_present() {
    let stdout = concat!(
        "MOTH_BENCH timing-schema 2\n",
        "MOTH_BENCH timing command.check.total=8ms\n",
        "MOTH_BENCH counter work=2\n",
        "MOTH_BENCH counter work=3\n",
        "MOTH_BENCH counter cache_hits=4.5\n",
    );

    let observations = parse_stdout_observations(stdout, CliBenchmarkCommand::Check)
        .expect("valid optional counters should parse");

    assert_metric_value(&observations.counters, "work", 5.0);
    assert_metric_value(&observations.counters, "cache_hits", 4.5);
}

#[test]
fn stable_records_allow_unrelated_surrounding_output_and_emitter_ansi_reset() {
    let stdout = concat!(
        "Checking project\n",
        "MOTH_BENCH timing-schema 2\u{1b}[0m\n",
        "MOTH_BENCH timing command.check.total=8ms\u{1b}[0m\n",
        "MOTH_BENCH counter work=3\u{1b}[0m\n",
        "Finished\n",
    );

    let observations = parse_stdout_observations(stdout, CliBenchmarkCommand::Check)
        .expect("stable records should parse after emitter ANSI normalization");

    assert_metric_value(&observations.stage_timings, "command.check.total", 8.0);
    assert_metric_value(&observations.counters, "work", 3.0);
}

#[test]
fn stable_timing_metric_set_mismatch_fails_before_averaging() {
    let observations = vec![
        observations(&[("command.check.total", 8.0), ("frontend.ast.total", 2.0)]),
        observations(&[("command.check.total", 9.0), ("frontend.hir", 3.0)]),
    ];

    let error = average_observations(&observations)
        .expect_err("missing and additional timing names must fail");

    assert_eq!(
        error,
        BenchmarkObservationError::TimingMetricSetMismatch {
            iteration: 2,
            missing: vec!["frontend.ast.total".to_owned()],
            additional: vec!["frontend.hir".to_owned()],
        }
    );
}

#[test]
fn averaging_rejects_duplicate_current_timing_aggregates() {
    let observations = vec![observations(&[
        ("command.check.total", 8.0),
        ("command.check.total", 2.0),
    ])];

    let error = average_observations(&observations)
        .expect_err("duplicate current timing aggregates must fail before averaging");
    assert_eq!(
        error,
        BenchmarkObservationError::DuplicateTimingMetric {
            metric_name: "command.check.total".to_owned()
        }
    );
}

#[test]
fn consistent_metric_sets_average_correctly() {
    let observations = vec![
        BenchmarkCaseObservations {
            timing_schema_version: BENCHMARK_TIMING_SCHEMA_VERSION,
            stage_timings: vec![
                metric("command.check.total", 8.0),
                metric("frontend.bind_headers", 2.0),
                metric("frontend.ast.total", 3.0),
            ],
            counters: vec![metric("work", 4.0)],
        },
        BenchmarkCaseObservations {
            timing_schema_version: BENCHMARK_TIMING_SCHEMA_VERSION,
            stage_timings: vec![
                metric("command.check.total", 12.0),
                metric("frontend.bind_headers", 7.0),
                metric("frontend.ast.total", 9.0),
            ],
            counters: Vec::new(),
        },
    ];

    let averaged =
        average_observations(&observations).expect("consistent metric sets should average");

    assert_metric_value(&averaged.stage_timings, "command.check.total", 10.0);
    assert_metric_value(&averaged.stage_timings, "frontend.bind_headers", 4.5);
    assert_metric_value(&averaged.stage_timings, "frontend.ast.total", 6.0);
    assert_metric_value(&averaged.counters, "work", 2.0);
}

#[test]
fn frontend_observations_require_valid_nonempty_stages_and_validate_counters() {
    assert_eq!(
        validate_frontend_observations(BenchmarkCaseObservations::default()),
        Err(BenchmarkObservationError::MissingFrontendStages)
    );

    for invalid_value in [f64::NAN, f64::INFINITY, -1.0] {
        let stage_error = validate_frontend_observations(BenchmarkCaseObservations {
            timing_schema_version: BENCHMARK_TIMING_SCHEMA_VERSION,
            stage_timings: vec![metric("frontend.ast.total", invalid_value)],
            counters: Vec::new(),
        })
        .expect_err("invalid frontend stage must fail");
        assert!(matches!(
            stage_error,
            BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "timing",
                ..
            }
        ));

        let counter_error = validate_frontend_observations(BenchmarkCaseObservations {
            timing_schema_version: BENCHMARK_TIMING_SCHEMA_VERSION,
            stage_timings: vec![metric("frontend.ast.total", 1.0)],
            counters: vec![metric("work", invalid_value)],
        })
        .expect_err("invalid frontend counter must fail");
        assert!(matches!(
            counter_error,
            BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "counter",
                ..
            }
        ));
    }
}

#[test]
fn malformed_stable_counter_line_fails() {
    let error = parse_stdout_observations(
        concat!(
            "MOTH_BENCH timing-schema 2\n",
            "MOTH_BENCH timing command.check.total=1ms\n",
            "MOTH_BENCH counter work=1 trailing\n",
        ),
        CliBenchmarkCommand::Check,
    )
    .expect_err("malformed counter must fail");

    assert!(matches!(
        error,
        BenchmarkObservationError::InvalidMetricValue {
            metric_kind: "counter",
            ..
        }
    ));
}

fn observations(stages: &[(&str, f64)]) -> BenchmarkCaseObservations {
    BenchmarkCaseObservations {
        timing_schema_version: BENCHMARK_TIMING_SCHEMA_VERSION,
        stage_timings: stages
            .iter()
            .map(|(name, value)| metric(name, *value))
            .collect(),
        counters: Vec::new(),
    }
}

fn metric(name: &str, value: f64) -> BenchmarkMetric {
    BenchmarkMetric {
        name: name.to_owned(),
        value,
    }
}

fn assert_metric_value(metrics: &[BenchmarkMetric], name: &str, expected: f64) {
    let metric = metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap_or_else(|| panic!("missing metric {name}"));

    assert!(
        (metric.value - expected).abs() < 0.000001,
        "expected {name} to be {expected}, got {}",
        metric.value
    );
}
