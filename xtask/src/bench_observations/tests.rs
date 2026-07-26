use super::*;

#[test]
fn parses_exact_live_check_and_build_metrics() {
    for (command, required_name) in [
        (CliBenchmarkCommand::Check, "command.check.total"),
        (CliBenchmarkCommand::Build, "command.build.total"),
    ] {
        let stdout = format!(
            "MOTH_BENCH timing frontend.ast=2.5ms\nMOTH_BENCH timing {required_name}=8ms\n"
        );

        let observations =
            parse_stdout_observations(&stdout, BenchmarkObservationSource::LiveCli(command))
                .expect("exact live timing records should parse");

        assert_metric_value(&observations.stage_timings, required_name, 8.0);
        assert_metric_value(&observations.stage_timings, "frontend.ast", 2.5);
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
        let error = parse_stdout_observations(
            line,
            BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
        )
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
fn non_finite_and_negative_values_fail() {
    for value in ["NaN", "inf", "-1"] {
        let timing_stdout = format!("MOTH_BENCH timing command.check.total={value}ms");
        let timing_error = parse_stdout_observations(
            &timing_stdout,
            BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
        )
        .expect_err("invalid timing values must fail");
        assert!(matches!(
            timing_error,
            BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "timing",
                ..
            }
        ));

        let counter_stdout =
            format!("MOTH_BENCH timing command.check.total=1ms\nMOTH_BENCH counter work={value}");
        let counter_error = parse_stdout_observations(
            &counter_stdout,
            BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
        )
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
        "MOTH_BENCH timing frontend.ast=1ms",
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
    )
    .expect_err("check total is required");
    assert_eq!(
        check_error,
        BenchmarkObservationError::MissingRequiredTiming {
            metric_name: "command.check.total"
        }
    );

    let build_error = parse_stdout_observations(
        "MOTH_BENCH timing command.check.total=1ms",
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Build),
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
fn repeated_timing_names_sum_within_one_iteration() {
    let stdout = concat!(
        "MOTH_BENCH timing frontend.ast=2ms\n",
        "MOTH_BENCH timing frontend.ast=3ms\n",
        "MOTH_BENCH timing command.check.total=8ms\n",
    );

    let observations = parse_stdout_observations(
        stdout,
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
    )
    .expect("repeated module metrics should parse");

    assert_metric_value(&observations.stage_timings, "frontend.ast", 5.0);
}

#[test]
fn stable_counters_parse_and_sum_when_present() {
    let stdout = concat!(
        "MOTH_BENCH timing command.check.total=8ms\n",
        "MOTH_BENCH counter work=2\n",
        "MOTH_BENCH counter work=3\n",
        "MOTH_BENCH counter cache_hits=4.5\n",
    );

    let observations = parse_stdout_observations(
        stdout,
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
    )
    .expect("valid optional counters should parse");

    assert_metric_value(&observations.counters, "work", 5.0);
    assert_metric_value(&observations.counters, "cache_hits", 4.5);
}

#[test]
fn stable_records_allow_unrelated_surrounding_output_and_emitter_ansi_reset() {
    let stdout = concat!(
        "Checking project\n",
        "MOTH_BENCH timing command.check.total=8ms\u{1b}[0m\n",
        "MOTH_BENCH counter work=3\u{1b}[0m\n",
        "Finished\n",
    );

    let observations = parse_stdout_observations(
        stdout,
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
    )
    .expect("stable records should parse after emitter ANSI normalization");

    assert_metric_value(&observations.stage_timings, "command.check.total", 8.0);
    assert_metric_value(&observations.counters, "work", 3.0);
}

#[test]
fn stable_timing_metric_set_mismatch_fails_before_averaging() {
    let observations = vec![
        observations(&[("command.check.total", 8.0), ("frontend.ast", 2.0)]),
        observations(&[("command.check.total", 9.0), ("frontend.hir", 3.0)]),
    ];

    let error = average_observations(&observations)
        .expect_err("missing and additional timing names must fail");

    assert_eq!(
        error,
        BenchmarkObservationError::TimingMetricSetMismatch {
            iteration: 2,
            missing: vec!["frontend.ast".to_owned()],
            additional: vec!["frontend.hir".to_owned()],
        }
    );
}

#[test]
fn consistent_metric_sets_average_correctly() {
    let observations = vec![
        BenchmarkCaseObservations {
            stage_timings: vec![
                metric("command.check.total", 8.0),
                metric("frontend.ast", 2.0),
                metric("frontend.ast", 3.0),
            ],
            counters: vec![metric("work", 4.0)],
        },
        BenchmarkCaseObservations {
            stage_timings: vec![
                metric("frontend.ast", 7.0),
                metric("command.check.total", 12.0),
            ],
            counters: Vec::new(),
        },
    ];

    let averaged =
        average_observations(&observations).expect("consistent metric sets should average");

    assert_metric_value(&averaged.stage_timings, "command.check.total", 10.0);
    assert_metric_value(&averaged.stage_timings, "frontend.ast", 6.0);
    assert_metric_value(&averaged.counters, "work", 2.0);
}

#[test]
fn legacy_prose_is_accepted_only_by_explicit_history_path() {
    let stdout = concat!(
        "AST created in: \u{1b}[32m2ms\u{1b}[0m\n",
        "HIR generated in: 3ms\n",
    );

    let live_error = parse_stdout_observations(
        stdout,
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
    )
    .expect_err("live execution must not accept legacy-only prose");
    assert_eq!(
        live_error,
        BenchmarkObservationError::MissingRequiredTiming {
            metric_name: "command.check.total"
        }
    );

    let legacy = parse_stdout_observations(stdout, BenchmarkObservationSource::LegacyHistory)
        .expect("explicit legacy history path should retain old prose");
    assert_metric_value(&legacy.stage_timings, "ast_ms", 2.0);
    assert_metric_value(&legacy.stage_timings, "hir_ms", 3.0);
}

#[test]
fn legacy_history_preserves_supported_duration_units() {
    let stdout = concat!(
        "Tokenized in: 335.834µs\n",
        "Headers Parsed in: 0.002s\n",
        "AST created in: 1ms\n",
    );

    let observations = parse_stdout_observations(stdout, BenchmarkObservationSource::LegacyHistory)
        .expect("legacy duration units should remain readable");

    assert_metric_value(&observations.stage_timings, "tokenize_ms", 0.335834);
    assert_metric_value(&observations.stage_timings, "headers_ms", 2.0);
    assert_metric_value(&observations.stage_timings, "ast_ms", 1.0);
}

#[test]
fn stable_metrics_take_precedence_only_in_legacy_history_path() {
    let stdout = concat!(
        "MOTH_BENCH timing ast_ms=10ms\n",
        "AST created in: 2ms\n",
        "HIR generated in: 3ms\n",
    );

    let observations = parse_stdout_observations(stdout, BenchmarkObservationSource::LegacyHistory)
        .expect("mixed old history should parse");

    assert_metric_value(&observations.stage_timings, "ast_ms", 10.0);
    assert_metric_value(&observations.stage_timings, "hir_ms", 3.0);
}

#[test]
fn frontend_observations_require_valid_nonempty_stages_and_validate_counters() {
    assert_eq!(
        validate_frontend_observations(BenchmarkCaseObservations::default()),
        Err(BenchmarkObservationError::MissingFrontendStages)
    );

    for invalid_value in [f64::NAN, f64::INFINITY, -1.0] {
        let stage_error = validate_frontend_observations(BenchmarkCaseObservations {
            stage_timings: vec![metric("frontend.ast", invalid_value)],
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
            stage_timings: vec![metric("frontend.ast", 1.0)],
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
            "MOTH_BENCH timing command.check.total=1ms\n",
            "MOTH_BENCH counter work=1 trailing\n",
        ),
        BenchmarkObservationSource::LiveCli(CliBenchmarkCommand::Check),
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
