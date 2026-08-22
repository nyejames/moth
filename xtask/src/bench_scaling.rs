//! Growth-exponent analysis for declared benchmark scaling series.
//!
//! WHAT: Runs the cases named by each manifest scaling series, fits the growth
//! exponent of that series' timing metric against the declared input size, and
//! holds the result to the series budget.
//! WHY: The normal suites compare a case against its own recorded history, so
//! they answer "did this change". A cost that has been superlinear since the day
//! it was written never changes, so it never reports. This lane asks the
//! question history cannot: how does this cost grow when the module grows.
//! MUST NOT: Write local history or tracked summaries. A scaling verdict is a
//! statement about program shape, not a recorded measurement of this machine.

use crate::bench_types::{BenchmarkCaseResult, BenchmarkRecording};
use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_manifest::{BenchmarkCase, BenchmarkScalingSeries};
use crate::benchmark_run::PreparedBenchmarkRun;
use crate::benchmark_suite::measure_cases;
use crate::benchmark_workspace::{BenchmarkExecutionWorkspace, finalise_workspace};
use crate::benchmark_repository::verify_after_operation;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

/// Measured iterations per scaling case.
///
/// WHY: the fit spans an order of magnitude of input size, so it tolerates more
/// per-case noise than a regression comparison does. Five keeps the lane quick
/// while still averaging away a single disturbed run.
const SCALING_MEASURED_ITERATIONS: usize = 5;

/// Smallest value the largest point may have before the fit is trusted, in ms.
///
/// WHY: an exponent fitted across sub-millisecond timings describes scheduler
/// noise, not the compiler. Reporting it as a confident number would be worse
/// than reporting nothing.
const MINIMUM_LARGEST_POINT_MS: f64 = 1.0;

/// One measured member of a scaling series.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScalingPointMeasurement {
    pub(crate) case_id: String,
    pub(crate) size: u32,
    pub(crate) metric_ms: f64,
}

/// The conclusion drawn about one series.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScalingVerdict {
    WithinBudget { exponent: f64 },
    ExceedsBudget { exponent: f64 },
    /// The series ran but the numbers cannot support a conclusion.
    Unmeasurable { reason: String },
}

/// One series, its measured points and its verdict.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScalingSeriesOutcome {
    pub(crate) id: String,
    pub(crate) metric: String,
    pub(crate) max_exponent: f64,
    pub(crate) points: Vec<ScalingPointMeasurement>,
    pub(crate) verdict: ScalingVerdict,
}

impl ScalingSeriesOutcome {
    pub(crate) fn failed(&self) -> bool {
        matches!(
            self.verdict,
            ScalingVerdict::ExceedsBudget { .. } | ScalingVerdict::Unmeasurable { .. }
        )
    }
}

/// Fit the exponent `k` in `time = c * size^k` by least squares in log-log space.
///
/// WHAT: returns the slope of `ln(metric)` regressed on `ln(size)`.
/// WHY: a single ratio between the smallest and largest point is decided by two
/// measurements. A least-squares slope uses every point, so one disturbed run
/// moves the answer instead of setting it.
///
/// Returns `None` when any point is non-positive or every size is identical,
/// because the logarithm or the slope would then be undefined.
pub(crate) fn fit_growth_exponent(points: &[ScalingPointMeasurement]) -> Option<f64> {
    if points.len() < 2 {
        return None;
    }

    let mut log_sizes = Vec::with_capacity(points.len());
    let mut log_metrics = Vec::with_capacity(points.len());
    for point in points {
        if point.size == 0 || point.metric_ms <= 0.0 {
            return None;
        }
        log_sizes.push(f64::from(point.size).ln());
        log_metrics.push(point.metric_ms.ln());
    }

    let count = points.len() as f64;
    let mean_log_size = log_sizes.iter().sum::<f64>() / count;
    let mean_log_metric = log_metrics.iter().sum::<f64>() / count;

    let mut covariance = 0.0;
    let mut size_variance = 0.0;
    for (log_size, log_metric) in log_sizes.iter().zip(log_metrics.iter()) {
        let size_deviation = log_size - mean_log_size;
        covariance += size_deviation * (log_metric - mean_log_metric);
        size_variance += size_deviation * size_deviation;
    }

    if size_variance <= 0.0 {
        return None;
    }

    Some(covariance / size_variance)
}

/// Draw the verdict for one series from its measured points.
pub(crate) fn evaluate_series(
    series: &BenchmarkScalingSeries,
    points: Vec<ScalingPointMeasurement>,
) -> ScalingSeriesOutcome {
    let verdict = scaling_verdict(series, &points);

    ScalingSeriesOutcome {
        id: series.id.clone(),
        metric: series.metric.clone(),
        max_exponent: series.max_exponent,
        points,
        verdict,
    }
}

fn scaling_verdict(
    series: &BenchmarkScalingSeries,
    points: &[ScalingPointMeasurement],
) -> ScalingVerdict {
    if let Some(missing) = points.iter().find(|point| point.metric_ms <= 0.0) {
        return ScalingVerdict::Unmeasurable {
            reason: format!(
                "case '{}' did not report metric '{}'; a Detailed metric is not emitted by the \
                 benchmark compiler, which is built with `timers` only",
                missing.case_id, series.metric
            ),
        };
    }

    let largest = points
        .last()
        .map(|point| point.metric_ms)
        .unwrap_or_default();
    if largest < MINIMUM_LARGEST_POINT_MS {
        return ScalingVerdict::Unmeasurable {
            reason: format!(
                "largest point measured {largest:.3}ms, below the {MINIMUM_LARGEST_POINT_MS:.3}ms \
                 floor; grow the fixture rather than trusting a fit over noise"
            ),
        };
    }

    let Some(exponent) = fit_growth_exponent(points) else {
        return ScalingVerdict::Unmeasurable {
            reason: "points do not support a log-log fit".to_owned(),
        };
    };

    if exponent > series.max_exponent {
        ScalingVerdict::ExceedsBudget { exponent }
    } else {
        ScalingVerdict::WithinBudget { exponent }
    }
}

/// Render one series as a table of points, ratios and the verdict.
///
/// WHY: the exponent alone is a number to argue with. The per-step ratios show
/// the shape a reader can recognise: doubling the input tripled the time.
pub(crate) fn format_series_report(outcome: &ScalingSeriesOutcome) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Scaling series '{}' — metric {} — budget n^{:.2}",
        outcome.id, outcome.metric, outcome.max_exponent
    ));
    lines.push(format!(
        "  {:>10}  {:>12}  {:>10}  {:>10}",
        "size", "metric_ms", "size step", "time step"
    ));

    let mut previous: Option<&ScalingPointMeasurement> = None;
    for point in &outcome.points {
        let (size_step, time_step) = match previous {
            Some(earlier) if earlier.size > 0 && earlier.metric_ms > 0.0 => (
                format!("{:.2}x", f64::from(point.size) / f64::from(earlier.size)),
                format!("{:.2}x", point.metric_ms / earlier.metric_ms),
            ),
            _ => ("-".to_owned(), "-".to_owned()),
        };
        lines.push(format!(
            "  {:>10}  {:>12.3}  {:>10}  {:>10}",
            point.size, point.metric_ms, size_step, time_step
        ));
        previous = Some(point);
    }

    let verdict_line = match &outcome.verdict {
        ScalingVerdict::WithinBudget { exponent } => {
            format!("  fitted n^{exponent:.2} — within budget")
        }
        ScalingVerdict::ExceedsBudget { exponent } => format!(
            "  fitted n^{exponent:.2} — EXCEEDS BUDGET n^{:.2}",
            outcome.max_exponent
        ),
        ScalingVerdict::Unmeasurable { reason } => format!("  UNMEASURABLE — {reason}"),
    };
    lines.push(verdict_line);

    lines.join("\n")
}

/// Read one metric value out of a measured case result.
fn metric_from_result(result: &BenchmarkCaseResult, metric: &str) -> f64 {
    result
        .observations
        .stage_timings
        .iter()
        .find(|timing| timing.name == metric)
        .map(|timing| timing.value)
        .unwrap_or_default()
}

/// Collect the measured points for one series, in declared size order.
fn series_points(
    series: &BenchmarkScalingSeries,
    results: &[BenchmarkCaseResult],
) -> Vec<ScalingPointMeasurement> {
    series
        .points
        .iter()
        .map(|point| {
            let metric_ms = results
                .iter()
                .find(|result| result.case_id == point.case_id)
                .map(|result| metric_from_result(result, &series.metric))
                .unwrap_or_default();

            ScalingPointMeasurement {
                case_id: point.case_id.clone(),
                size: point.size,
                metric_ms,
            }
        })
        .collect()
}

/// Run every declared scaling series and report each verdict.
pub(crate) fn run_scaling_benchmarks() -> Result<(), String> {
    let prepared = PreparedBenchmarkRun::load(BenchmarkRecording::ReadOnly)?;

    if prepared.manifest.scaling_series.is_empty() {
        return Err("The manifest declares no scaling series.".to_owned());
    }

    let member_case_ids: BTreeSet<&str> = prepared
        .manifest
        .scaling_series
        .iter()
        .flat_map(|series| series.points.iter())
        .map(|point| point.case_id.as_str())
        .collect();

    let cases: Vec<BenchmarkCase> = prepared
        .manifest
        .cases
        .iter()
        .filter(|case| member_case_ids.contains(case.id.as_str()))
        .cloned()
        .collect();

    let workspace = BenchmarkExecutionWorkspace::create(&prepared.manifest.repository_root)?;
    let context = BenchmarkExecutionContext::frontend(&prepared.manifest, &workspace);

    println!(
        "Running {} scaling case(s) across {} series: 1 shared preflight + {} measured",
        cases.len(),
        prepared.manifest.scaling_series.len(),
        SCALING_MEASURED_ITERATIONS
    );

    let iterations = NonZeroUsize::new(SCALING_MEASURED_ITERATIONS)
        .ok_or_else(|| "scaling iterations must be nonzero".to_owned())?;

    let measured = (|| {
        preflight_cases(&context, &cases)
            .map_err(|failures| format_case_failures("preflight", &failures))?;
        println!("Shared scaling preflight passed; starting measurements.");
        measure_cases(&context, &prepared, &cases, iterations)
    })();

    let result = match measured {
        Ok(case_results) => {
            finalise_workspace(&workspace, Ok(()))?;
            report_scaling_outcomes(&prepared.manifest.scaling_series, &case_results)
        }
        Err(operation) => finalise_workspace(&workspace, Err(operation)),
    };

    verify_after_operation(
        &prepared.snapshot,
        &prepared.manifest.repository_root,
        result,
    )
}

/// Print every series verdict and fail the command if any series failed.
fn report_scaling_outcomes(
    series_list: &[BenchmarkScalingSeries],
    case_results: &[BenchmarkCaseResult],
) -> Result<(), String> {
    let outcomes: Vec<ScalingSeriesOutcome> = series_list
        .iter()
        .map(|series| evaluate_series(series, series_points(series, case_results)))
        .collect();

    println!();
    for outcome in &outcomes {
        println!("{}", format_series_report(outcome));
        println!();
    }

    let failed: Vec<&ScalingSeriesOutcome> =
        outcomes.iter().filter(|outcome| outcome.failed()).collect();

    if failed.is_empty() {
        println!("All {} scaling series within budget.", outcomes.len());
        return Ok(());
    }

    let names: Vec<&str> = failed.iter().map(|outcome| outcome.id.as_str()).collect();
    Err(format!(
        "{} of {} scaling series failed: {}",
        failed.len(),
        outcomes.len(),
        names.join(", ")
    ))
}

#[cfg(test)]
mod tests;
