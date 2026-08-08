//! Checked benchmark observation parsing and aggregation.
//!
//! WHAT: validates stable live timing and counter records, and checks measured
//! metric sets before averaging.
//! WHY: malformed or incomplete timing evidence must stop a benchmark before
//! its result can reach local history or tracked summaries.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{Display, Formatter};

use crate::bench_types::{
    BENCHMARK_TIMING_SCHEMA_VERSION, BenchmarkCaseObservations, BenchmarkMetric,
};
use crate::benchmark_manifest::{BenchmarkRunner, CliBenchmarkCommand};

const STABLE_TIMING_PREFIX: &str = "MOTH_BENCH timing";
const STABLE_TIMING_FIELDS_PREFIX: &str = "MOTH_BENCH timing ";
const STABLE_TIMING_SCHEMA_PREFIX: &str = "MOTH_BENCH timing-schema ";
const STABLE_COUNTER_PREFIX: &str = "MOTH_BENCH counter";
const STABLE_COUNTER_FIELDS_PREFIX: &str = "MOTH_BENCH counter ";

/// A malformed or internally inconsistent benchmark observation set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BenchmarkObservationError {
    MalformedTimingSchemaLine {
        line: String,
    },
    DuplicateTimingSchema,
    MissingTimingSchema,
    UnsupportedTimingSchema {
        version: u32,
    },
    TimingSchemaMismatch {
        expected: u32,
        actual: u32,
    },
    DuplicateTimingMetric {
        metric_name: String,
    },
    MalformedTimingLine {
        line: String,
    },
    MalformedCounterLine {
        line: String,
    },
    InvalidMetricName {
        metric_kind: &'static str,
        metric_name: String,
    },
    UnknownTimingMetric {
        metric_name: String,
    },
    TimingMetricOutOfOrder {
        previous_metric_name: String,
        metric_name: String,
    },
    InvalidMetricValue {
        metric_kind: &'static str,
        metric_name: String,
        value: String,
    },
    MissingRequiredTiming {
        metric_name: &'static str,
    },
    MissingTimingMetrics,
    MissingFrontendStages,
    NoMeasuredIterations,
    TimingMetricSetMismatch {
        iteration: usize,
        missing: Vec<String>,
        additional: Vec<String>,
    },
    MetricSumNotFinite {
        metric_kind: &'static str,
        metric_name: String,
    },
}

impl Display for BenchmarkObservationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedTimingSchemaLine { line } => {
                write!(
                    formatter,
                    "malformed MOTH_BENCH timing-schema record: {line}"
                )
            }
            Self::DuplicateTimingSchema => {
                write!(
                    formatter,
                    "found more than one MOTH_BENCH timing-schema record"
                )
            }
            Self::MissingTimingSchema => {
                write!(formatter, "missing MOTH_BENCH timing-schema record")
            }
            Self::UnsupportedTimingSchema { version } => {
                write!(
                    formatter,
                    "unsupported MOTH_BENCH timing schema version {version}"
                )
            }
            Self::TimingSchemaMismatch { expected, actual } => {
                write!(
                    formatter,
                    "timing schema changed: expected {expected}, got {actual}"
                )
            }
            Self::DuplicateTimingMetric { metric_name } => {
                write!(
                    formatter,
                    "timing metric '{metric_name}' was emitted more than once"
                )
            }
            Self::MalformedTimingLine { line } => {
                write!(formatter, "malformed MOTH_BENCH timing record: {line}")
            }
            Self::MalformedCounterLine { line } => {
                write!(formatter, "malformed MOTH_BENCH counter record: {line}")
            }
            Self::InvalidMetricName {
                metric_kind,
                metric_name,
            } => {
                write!(
                    formatter,
                    "{metric_kind} metric name must be non-empty and contain no whitespace or control characters, got '{metric_name}'"
                )
            }
            Self::UnknownTimingMetric { metric_name } => {
                write!(formatter, "unknown timing schema metric '{metric_name}'")
            }
            Self::TimingMetricOutOfOrder {
                previous_metric_name,
                metric_name,
            } => write!(
                formatter,
                "timing metric '{metric_name}' was emitted after '{previous_metric_name}' outside schema order"
            ),
            Self::InvalidMetricValue {
                metric_kind,
                metric_name,
                value,
            } => {
                write!(
                    formatter,
                    "{metric_kind} metric '{metric_name}' must be finite and non-negative, got {value}"
                )
            }
            Self::MissingRequiredTiming { metric_name } => {
                write!(formatter, "missing required timing metric '{metric_name}'")
            }
            Self::MissingTimingMetrics => {
                write!(formatter, "current timing evidence contains no metrics")
            }
            Self::MissingFrontendStages => {
                write!(
                    formatter,
                    "frontend observation must contain at least one stage"
                )
            }
            Self::NoMeasuredIterations => {
                write!(formatter, "cannot average zero measured observations")
            }
            Self::TimingMetricSetMismatch {
                iteration,
                missing,
                additional,
            } => {
                write!(
                    formatter,
                    "timing metric set changed in measured iteration {iteration}"
                )?;
                if !missing.is_empty() {
                    write!(formatter, "; missing: {}", missing.join(", "))?;
                }
                if !additional.is_empty() {
                    write!(formatter, "; additional: {}", additional.join(", "))?;
                }
                Ok(())
            }
            Self::MetricSumNotFinite {
                metric_kind,
                metric_name,
            } => {
                write!(
                    formatter,
                    "summed {metric_kind} metric '{metric_name}' is not finite"
                )
            }
        }
    }
}

impl std::error::Error for BenchmarkObservationError {}

/// Parse checked stdout observations for one CLI command.
///
/// Live output accepts stable schema-v1 records only and requires the
/// command's top-level timing. Pre-v1 history is decoded by the history owner
/// as schema-less legacy data and never enters this parser.
pub(crate) fn parse_stdout_observations(
    stdout: &str,
    command: CliBenchmarkCommand,
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    let mut timing_schema_version = None;
    let mut stable_timings = Vec::new();
    let mut counters = Vec::new();

    for raw_line in stdout.lines() {
        let line = strip_ansi_codes(raw_line);

        if line.starts_with(STABLE_TIMING_SCHEMA_PREFIX) {
            if timing_schema_version.is_some() {
                return Err(BenchmarkObservationError::DuplicateTimingSchema);
            }
            let version = parse_timing_schema_line(&line)?;
            if version != BENCHMARK_TIMING_SCHEMA_VERSION {
                return Err(BenchmarkObservationError::UnsupportedTimingSchema { version });
            }
            timing_schema_version = Some(version);
            continue;
        }

        if line.starts_with(STABLE_TIMING_PREFIX) {
            stable_timings.push(parse_stable_timing_line(&line)?);
            continue;
        }

        if line.starts_with(STABLE_COUNTER_PREFIX) {
            counters.push(parse_stable_counter_line(&line)?);
            continue;
        }
    }

    if timing_schema_version.is_none() {
        return Err(BenchmarkObservationError::MissingTimingSchema);
    }

    validate_current_timing_metrics(&stable_timings)?;

    let stage_timings = sum_metrics_by_name(stable_timings, "timing")?;

    let observations = BenchmarkCaseObservations {
        timing_schema_version: timing_schema_version.unwrap_or(0),
        stage_timings,
        counters: sum_metrics_by_name(counters, "counter")?,
    };

    require_cli_total(&observations, command)?;

    Ok(observations)
}

/// Validate and normalize one in-process frontend observation report.
pub(crate) fn validate_frontend_observations(
    observations: BenchmarkCaseObservations,
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    if observations.stage_timings.is_empty() {
        return Err(BenchmarkObservationError::MissingFrontendStages);
    }
    if observations.timing_schema_version != BENCHMARK_TIMING_SCHEMA_VERSION {
        return Err(BenchmarkObservationError::TimingSchemaMismatch {
            expected: BENCHMARK_TIMING_SCHEMA_VERSION,
            actual: observations.timing_schema_version,
        });
    }
    validate_current_timing_metrics(&observations.stage_timings)?;

    normalize_observations(observations)
}

/// Average measured observations only after their timing metric sets agree.
pub(crate) fn average_observations(
    observations: &[BenchmarkCaseObservations],
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    if observations.is_empty() {
        return Err(BenchmarkObservationError::NoMeasuredIterations);
    }

    let mut normalized = Vec::with_capacity(observations.len());
    for observation in observations {
        if observation.timing_schema_version == BENCHMARK_TIMING_SCHEMA_VERSION {
            validate_current_timing_metrics(&observation.stage_timings)?;
        }
        normalized.push(normalize_observations(observation.clone())?);
    }

    let timing_schema_version = normalized[0].timing_schema_version;
    if timing_schema_version != BENCHMARK_TIMING_SCHEMA_VERSION {
        return Err(BenchmarkObservationError::TimingSchemaMismatch {
            expected: BENCHMARK_TIMING_SCHEMA_VERSION,
            actual: timing_schema_version,
        });
    }
    for observation in normalized.iter().skip(1) {
        if observation.timing_schema_version != timing_schema_version {
            return Err(BenchmarkObservationError::TimingSchemaMismatch {
                expected: timing_schema_version,
                actual: observation.timing_schema_version,
            });
        }
    }

    let expected_timing_names = metric_name_set(&normalized[0].stage_timings);
    for (index, observation) in normalized.iter().enumerate().skip(1) {
        let timing_names = metric_name_set(&observation.stage_timings);
        if timing_names == expected_timing_names {
            continue;
        }

        let missing = expected_timing_names
            .difference(&timing_names)
            .cloned()
            .collect();
        let additional = timing_names
            .difference(&expected_timing_names)
            .cloned()
            .collect();

        return Err(BenchmarkObservationError::TimingMetricSetMismatch {
            iteration: index + 1,
            missing,
            additional,
        });
    }

    let iteration_count = normalized.len();

    Ok(BenchmarkCaseObservations {
        timing_schema_version,
        stage_timings: average_metrics(
            normalized.iter().map(|item| &item.stage_timings),
            iteration_count,
            "timing",
        )?,
        // Optional counters behave as zero when an iteration did not emit them.
        counters: average_metrics(
            normalized.iter().map(|item| &item.counters),
            iteration_count,
            "counter",
        )?,
    })
}

fn require_cli_total(
    observations: &BenchmarkCaseObservations,
    command: CliBenchmarkCommand,
) -> Result<(), BenchmarkObservationError> {
    let required_name = match command {
        CliBenchmarkCommand::Check => moth::benchmarking::TIMING_COMMAND_CHECK_TOTAL_NAME,
        CliBenchmarkCommand::Build => moth::benchmarking::TIMING_COMMAND_BUILD_TOTAL_NAME,
    };

    if observations
        .stage_timings
        .iter()
        .any(|metric| metric.name == required_name)
    {
        Ok(())
    } else {
        Err(BenchmarkObservationError::MissingRequiredTiming {
            metric_name: required_name,
        })
    }
}

fn normalize_observations(
    observations: BenchmarkCaseObservations,
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    Ok(BenchmarkCaseObservations {
        timing_schema_version: observations.timing_schema_version,
        stage_timings: sum_metrics_by_name(observations.stage_timings, "timing")?,
        counters: sum_metrics_by_name(observations.counters, "counter")?,
    })
}

fn parse_timing_schema_line(line: &str) -> Result<u32, BenchmarkObservationError> {
    let version_text = line
        .strip_prefix(STABLE_TIMING_SCHEMA_PREFIX)
        .filter(|text| !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit()))
        .ok_or_else(|| BenchmarkObservationError::MalformedTimingSchemaLine {
            line: line.to_owned(),
        })?;

    version_text
        .parse::<u32>()
        .map_err(|_| BenchmarkObservationError::MalformedTimingSchemaLine {
            line: line.to_owned(),
        })
}

pub(crate) fn validate_current_timing_metrics(
    metrics: &[BenchmarkMetric],
) -> Result<(), BenchmarkObservationError> {
    validate_current_timing_metric_names(metrics.iter().map(|metric| metric.name.as_str()))
}

/// Validate the complete evidence contract for one current-schema persistence case.
///
/// Live observation paths perform their own runner-specific checks while parsing. History
/// readers and writers repeat the contract at the persistence boundary so incomplete current
/// records cannot become durable evidence after a hand-written or corrupted JSONL edit.
pub(crate) fn validate_current_timing_evidence<'a, I>(
    metric_names: I,
    required_metric_name: Option<&'static str>,
) -> Result<(), BenchmarkObservationError>
where
    I: IntoIterator<Item = &'a str>,
{
    let metric_names = metric_names.into_iter().collect::<Vec<_>>();
    if metric_names.is_empty() {
        return Err(BenchmarkObservationError::MissingTimingMetrics);
    }

    validate_current_timing_metric_names(metric_names.iter().copied())?;
    if let Some(required_metric_name) = required_metric_name
        && !metric_names.contains(&required_metric_name)
    {
        return Err(BenchmarkObservationError::MissingRequiredTiming {
            metric_name: required_metric_name,
        });
    }

    Ok(())
}

/// Return the command-total identity required by a typed benchmark runner.
pub(crate) fn required_command_total_for_runner(runner: &BenchmarkRunner) -> Option<&'static str> {
    match runner {
        BenchmarkRunner::Cli { command, .. } => Some(match command {
            CliBenchmarkCommand::Check => moth::benchmarking::TIMING_COMMAND_CHECK_TOTAL_NAME,
            CliBenchmarkCommand::Build => moth::benchmarking::TIMING_COMMAND_BUILD_TOTAL_NAME,
        }),
        BenchmarkRunner::Frontend { .. } => None,
    }
}

/// Return the command-total identity required by a persisted profile case.
pub(crate) fn required_command_total_for_name(command: &str) -> Option<&'static str> {
    match command {
        "check" => Some(moth::benchmarking::TIMING_COMMAND_CHECK_TOTAL_NAME),
        "build" => Some(moth::benchmarking::TIMING_COMMAND_BUILD_TOTAL_NAME),
        _ => None,
    }
}

/// Validate current-schema timing identities independent of the persistence
/// record type that carries their names.
pub(crate) fn validate_current_timing_metric_names<'a, I>(
    metric_names: I,
) -> Result<(), BenchmarkObservationError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut names = HashSet::new();
    let mut previous = None;
    for metric_name in metric_names {
        if !names.insert(metric_name) {
            return Err(BenchmarkObservationError::DuplicateTimingMetric {
                metric_name: metric_name.to_owned(),
            });
        }

        let Some(index) = moth::benchmarking::TIMING_SCHEMA_METRIC_NAMES
            .iter()
            .position(|name| *name == metric_name)
        else {
            return Err(BenchmarkObservationError::UnknownTimingMetric {
                metric_name: metric_name.to_owned(),
            });
        };

        if let Some((previous_index, previous_name)) = previous
            && index < previous_index
        {
            return Err(BenchmarkObservationError::TimingMetricOutOfOrder {
                previous_metric_name: previous_name,
                metric_name: metric_name.to_owned(),
            });
        }
        previous = Some((index, metric_name.to_owned()));
    }

    Ok(())
}

fn parse_stable_timing_line(line: &str) -> Result<BenchmarkMetric, BenchmarkObservationError> {
    let fields = line
        .strip_prefix(STABLE_TIMING_FIELDS_PREFIX)
        .ok_or_else(|| BenchmarkObservationError::MalformedTimingLine {
            line: line.to_owned(),
        })?;
    let (name, value_with_unit) = split_exact_metric_fields(fields).ok_or_else(|| {
        BenchmarkObservationError::MalformedTimingLine {
            line: line.to_owned(),
        }
    })?;

    validate_metric_name("timing", name)?;

    let value_text = value_with_unit.strip_suffix("ms").ok_or_else(|| {
        BenchmarkObservationError::MalformedTimingLine {
            line: line.to_owned(),
        }
    })?;
    if value_text.is_empty() || value_text.trim() != value_text {
        return Err(BenchmarkObservationError::MalformedTimingLine {
            line: line.to_owned(),
        });
    }

    let value =
        value_text
            .parse::<f64>()
            .map_err(|_| BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "timing",
                metric_name: name.to_owned(),
                value: value_text.to_owned(),
            })?;
    validate_metric_value("timing", name, value, value_text)?;

    Ok(BenchmarkMetric {
        name: name.to_owned(),
        value,
    })
}

fn parse_stable_counter_line(line: &str) -> Result<BenchmarkMetric, BenchmarkObservationError> {
    let fields = line
        .strip_prefix(STABLE_COUNTER_FIELDS_PREFIX)
        .ok_or_else(|| BenchmarkObservationError::MalformedCounterLine {
            line: line.to_owned(),
        })?;
    let (name, value_text) = split_exact_metric_fields(fields).ok_or_else(|| {
        BenchmarkObservationError::MalformedCounterLine {
            line: line.to_owned(),
        }
    })?;

    validate_metric_name("counter", name)?;

    let value =
        value_text
            .parse::<f64>()
            .map_err(|_| BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "counter",
                metric_name: name.to_owned(),
                value: value_text.to_owned(),
            })?;
    validate_metric_value("counter", name, value, value_text)?;

    Ok(BenchmarkMetric {
        name: name.to_owned(),
        value,
    })
}

fn split_exact_metric_fields(fields: &str) -> Option<(&str, &str)> {
    let (name, value) = fields.split_once('=')?;
    if name.trim() != name || value.trim() != value || value.contains('=') || value.is_empty() {
        return None;
    }

    Some((name, value))
}

fn validate_metric_name(
    metric_kind: &'static str,
    name: &str,
) -> Result<(), BenchmarkObservationError> {
    if name.is_empty() || name.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        Err(BenchmarkObservationError::InvalidMetricName {
            metric_kind,
            metric_name: name.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn validate_metric_value(
    metric_kind: &'static str,
    metric_name: &str,
    value: f64,
    rendered_value: &str,
) -> Result<(), BenchmarkObservationError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(BenchmarkObservationError::InvalidMetricValue {
            metric_kind,
            metric_name: metric_name.to_owned(),
            value: rendered_value.to_owned(),
        })
    }
}

fn average_metrics<'a>(
    metrics_by_iteration: impl Iterator<Item = &'a Vec<BenchmarkMetric>>,
    iteration_count: usize,
    metric_kind: &'static str,
) -> Result<Vec<BenchmarkMetric>, BenchmarkObservationError> {
    let mut indices_by_name: HashMap<String, usize> = HashMap::new();
    let mut sums = Vec::new();

    for metrics in metrics_by_iteration {
        for metric in metrics {
            let index = match indices_by_name.get(&metric.name) {
                Some(index) => *index,
                None => {
                    let index = sums.len();
                    indices_by_name.insert(metric.name.clone(), index);
                    sums.push(BenchmarkMetric {
                        name: metric.name.clone(),
                        value: 0.0,
                    });
                    index
                }
            };
            sums[index].value += metric.value;
            if !sums[index].value.is_finite() {
                return Err(BenchmarkObservationError::MetricSumNotFinite {
                    metric_kind,
                    metric_name: metric.name.clone(),
                });
            }
        }
    }

    for metric in &mut sums {
        metric.value /= iteration_count as f64;
    }
    Ok(sums)
}

fn sum_metrics_by_name(
    metrics: Vec<BenchmarkMetric>,
    metric_kind: &'static str,
) -> Result<Vec<BenchmarkMetric>, BenchmarkObservationError> {
    let mut indices_by_name: HashMap<String, usize> = HashMap::new();
    let mut sums = Vec::new();

    for metric in metrics {
        validate_metric_name(metric_kind, &metric.name)?;
        validate_metric_value(
            metric_kind,
            &metric.name,
            metric.value,
            &metric.value.to_string(),
        )?;

        let index = match indices_by_name.get(&metric.name) {
            Some(index) => *index,
            None => {
                let index = sums.len();
                indices_by_name.insert(metric.name.clone(), index);
                sums.push(BenchmarkMetric {
                    name: metric.name.clone(),
                    value: 0.0,
                });
                index
            }
        };
        sums[index].value += metric.value;
        if !sums[index].value.is_finite() {
            return Err(BenchmarkObservationError::MetricSumNotFinite {
                metric_kind,
                metric_name: metric.name,
            });
        }
    }

    Ok(sums)
}

fn metric_name_set(metrics: &[BenchmarkMetric]) -> BTreeSet<String> {
    metrics.iter().map(|metric| metric.name.clone()).collect()
}

fn strip_ansi_codes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            continue;
        }

        if let Some(ch) = text[index..].chars().next() {
            output.push(ch);
            index += ch.len_utf8();
        } else {
            break;
        }
    }

    output
}

#[cfg(test)]
mod tests;
