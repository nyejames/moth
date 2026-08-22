use super::*;
use crate::benchmark_manifest::BenchmarkScalingPoint;

fn series(max_exponent: f64) -> BenchmarkScalingSeries {
    BenchmarkScalingSeries {
        id: "example".to_owned(),
        metric: "frontend.ast.environment".to_owned(),
        max_exponent,
        points: vec![
            BenchmarkScalingPoint {
                case_id: "small".to_owned(),
                case_index: 0,
                size: 40,
            },
            BenchmarkScalingPoint {
                case_id: "medium".to_owned(),
                case_index: 1,
                size: 80,
            },
            BenchmarkScalingPoint {
                case_id: "large".to_owned(),
                case_index: 2,
                size: 160,
            },
        ],
    }
}

fn measured(values: [(u32, f64); 3]) -> Vec<ScalingPointMeasurement> {
    values
        .into_iter()
        .map(|(size, metric_ms)| ScalingPointMeasurement {
            case_id: format!("case_{size}"),
            size,
            metric_ms,
        })
        .collect()
}

#[test]
fn linear_growth_fits_an_exponent_of_one() {
    let points = measured([(40, 10.0), (80, 20.0), (160, 40.0)]);

    let exponent = fit_growth_exponent(&points).expect("linear points are fittable");

    assert!(
        (exponent - 1.0).abs() < 1e-9,
        "expected n^1.00, got n^{exponent}"
    );
}

#[test]
fn quadratic_growth_fits_an_exponent_of_two() {
    let points = measured([(40, 10.0), (80, 40.0), (160, 160.0)]);

    let exponent = fit_growth_exponent(&points).expect("quadratic points are fittable");

    assert!(
        (exponent - 2.0).abs() < 1e-9,
        "expected n^2.00, got n^{exponent}"
    );
}

#[test]
fn a_flat_metric_fits_an_exponent_of_zero() {
    let points = measured([(40, 12.0), (80, 12.0), (160, 12.0)]);

    let exponent = fit_growth_exponent(&points).expect("flat points are fittable");

    assert!(exponent.abs() < 1e-9, "expected n^0.00, got n^{exponent}");
}

#[test]
fn a_non_positive_measurement_is_not_fittable() {
    let points = measured([(40, 10.0), (80, 0.0), (160, 40.0)]);

    assert_eq!(fit_growth_exponent(&points), None);
}

#[test]
fn quadratic_growth_exceeds_a_near_linear_budget() {
    let outcome = evaluate_series(&series(1.25), measured([(40, 10.0), (80, 40.0), (160, 160.0)]));

    match outcome.verdict {
        ScalingVerdict::ExceedsBudget { exponent } => {
            assert!((exponent - 2.0).abs() < 1e-9);
        }
        other => panic!("expected ExceedsBudget, got {other:?}"),
    }
    assert!(outcome.failed());
}

#[test]
fn linear_growth_stays_within_a_near_linear_budget() {
    let outcome = evaluate_series(&series(1.25), measured([(40, 10.0), (80, 20.0), (160, 40.0)]));

    match outcome.verdict {
        ScalingVerdict::WithinBudget { exponent } => {
            assert!((exponent - 1.0).abs() < 1e-9);
        }
        other => panic!("expected WithinBudget, got {other:?}"),
    }
    assert!(!outcome.failed());
}

#[test]
fn a_missing_metric_is_reported_as_unmeasurable_and_fails() {
    let outcome = evaluate_series(&series(1.25), measured([(40, 10.0), (80, 0.0), (160, 40.0)]));

    match &outcome.verdict {
        ScalingVerdict::Unmeasurable { reason } => {
            assert!(
                reason.contains("did not report metric"),
                "unhelpful reason: {reason}"
            );
        }
        other => panic!("expected Unmeasurable, got {other:?}"),
    }
    // A series that cannot be measured must not pass silently.
    assert!(outcome.failed());
}

#[test]
fn timings_below_the_noise_floor_are_refused_rather_than_fitted() {
    let outcome = evaluate_series(&series(1.25), measured([(40, 0.05), (80, 0.2), (160, 0.8)]));

    match &outcome.verdict {
        ScalingVerdict::Unmeasurable { reason } => {
            assert!(reason.contains("floor"), "unhelpful reason: {reason}");
        }
        other => panic!("expected Unmeasurable, got {other:?}"),
    }
    assert!(outcome.failed());
}

#[test]
fn the_report_shows_per_step_ratios_and_the_verdict() {
    let outcome = evaluate_series(&series(1.25), measured([(40, 10.0), (80, 40.0), (160, 160.0)]));

    let report = format_series_report(&outcome);

    assert!(report.contains("2.00x"), "missing size step: {report}");
    assert!(report.contains("4.00x"), "missing time step: {report}");
    assert!(report.contains("EXCEEDS BUDGET"), "missing verdict: {report}");
}
