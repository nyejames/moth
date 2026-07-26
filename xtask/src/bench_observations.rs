//! Checked benchmark observation parsing and aggregation.
//!
//! WHAT: validates stable live timing and counter records, retains an explicit
//! legacy-history parser, and checks measured metric sets before averaging.
//! WHY: malformed or incomplete timing evidence must stop a benchmark before
//! its result can reach local history or tracked summaries.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::{Display, Formatter};

use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric};
use crate::benchmark_manifest::CliBenchmarkCommand;

const LEGACY_STAGE_PREFIXES: [(&str, &str); 10] = [
    ("Tokenized in:", "tokenize_ms"),
    ("Headers Parsed in:", "headers_ms"),
    ("Files Prepared in:", "file_prepare_ms"),
    ("Dependency graph created in:", "dependency_sort_ms"),
    ("AST created in:", "ast_ms"),
    ("HIR generated in:", "hir_ms"),
    ("Borrow checking completed in:", "borrow_ms"),
    (
        "AST/build environment completed in:",
        "ast_build_environment_ms",
    ),
    ("AST/emit nodes completed in:", "ast_emit_nodes_ms"),
    ("AST/finalize completed in:", "ast_finalize_ms"),
];

const STABLE_TIMING_PREFIX: &str = "MOTH_BENCH timing";
const STABLE_TIMING_FIELDS_PREFIX: &str = "MOTH_BENCH timing ";
const STABLE_COUNTER_PREFIX: &str = "MOTH_BENCH counter";
const STABLE_COUNTER_FIELDS_PREFIX: &str = "MOTH_BENCH counter ";

/// Selects whether observations come from a live command or old captured output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BenchmarkObservationSource {
    LiveCli(CliBenchmarkCommand),
    LegacyHistory,
}

/// A malformed or internally inconsistent benchmark observation set.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BenchmarkObservationError {
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
    InvalidMetricValue {
        metric_kind: &'static str,
        metric_name: String,
        value: String,
    },
    MissingRequiredTiming {
        metric_name: &'static str,
    },
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

/// Parse checked stdout observations for one explicit source.
///
/// Live CLI output accepts stable records only and requires the command's
/// top-level timing. Legacy history may additionally recover known human
/// timing prose, with stable records taking precedence for matching names.
pub(crate) fn parse_stdout_observations(
    stdout: &str,
    source: BenchmarkObservationSource,
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    let mut stable_timings = Vec::new();
    let mut legacy_timings = Vec::new();
    let mut counters = Vec::new();

    for raw_line in stdout.lines() {
        let line = strip_ansi_codes(raw_line);

        if line.starts_with(STABLE_TIMING_PREFIX) {
            stable_timings.push(parse_stable_timing_line(&line)?);
            continue;
        }

        if line.starts_with(STABLE_COUNTER_PREFIX) {
            counters.push(parse_stable_counter_line(&line)?);
            continue;
        }

        if source == BenchmarkObservationSource::LegacyHistory
            && let Some(legacy) = parse_legacy_stage_timing(line.trim())?
        {
            legacy_timings.push(legacy);
        }
    }

    let mut stage_timings = sum_metrics_by_name(stable_timings, "timing")?;

    if source == BenchmarkObservationSource::LegacyHistory {
        let stable_names: HashSet<&str> = stage_timings
            .iter()
            .map(|metric| metric.name.as_str())
            .collect();
        legacy_timings.retain(|metric| !stable_names.contains(metric.name.as_str()));
        stage_timings.extend(sum_metrics_by_name(legacy_timings, "timing")?);
        stage_timings.sort_by(|left, right| left.name.cmp(&right.name));
    }

    let observations = BenchmarkCaseObservations {
        stage_timings,
        counters: sum_metrics_by_name(counters, "counter")?,
    };

    if let BenchmarkObservationSource::LiveCli(command) = source {
        require_cli_total(&observations, command)?;
    }

    Ok(observations)
}

/// Validate and normalize one in-process frontend observation report.
pub(crate) fn validate_frontend_observations(
    observations: BenchmarkCaseObservations,
) -> Result<BenchmarkCaseObservations, BenchmarkObservationError> {
    if observations.stage_timings.is_empty() {
        return Err(BenchmarkObservationError::MissingFrontendStages);
    }

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
        normalized.push(normalize_observations(observation.clone())?);
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
        CliBenchmarkCommand::Check => "command.check.total",
        CliBenchmarkCommand::Build => "command.build.total",
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
        stage_timings: sum_metrics_by_name(observations.stage_timings, "timing")?,
        counters: sum_metrics_by_name(observations.counters, "counter")?,
    })
}

fn parse_legacy_stage_timing(
    line: &str,
) -> Result<Option<BenchmarkMetric>, BenchmarkObservationError> {
    for (prefix, name) in LEGACY_STAGE_PREFIXES {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };

        let value = parse_legacy_duration_to_ms(rest.trim()).ok_or_else(|| {
            BenchmarkObservationError::InvalidMetricValue {
                metric_kind: "timing",
                metric_name: name.to_owned(),
                value: rest.trim().to_owned(),
            }
        })?;
        validate_metric_value("timing", name, value, rest.trim())?;

        return Ok(Some(BenchmarkMetric {
            name: name.to_owned(),
            value,
        }));
    }

    Ok(None)
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
    let mut sums_by_name: BTreeMap<String, f64> = BTreeMap::new();

    for metrics in metrics_by_iteration {
        for metric in metrics {
            let sum = sums_by_name.entry(metric.name.clone()).or_default();
            *sum += metric.value;
            if !sum.is_finite() {
                return Err(BenchmarkObservationError::MetricSumNotFinite {
                    metric_kind,
                    metric_name: metric.name.clone(),
                });
            }
        }
    }

    Ok(sums_by_name
        .into_iter()
        .map(|(name, sum)| BenchmarkMetric {
            name,
            value: sum / iteration_count as f64,
        })
        .collect())
}

fn sum_metrics_by_name(
    metrics: Vec<BenchmarkMetric>,
    metric_kind: &'static str,
) -> Result<Vec<BenchmarkMetric>, BenchmarkObservationError> {
    let mut sums_by_name: BTreeMap<String, f64> = BTreeMap::new();

    for metric in metrics {
        validate_metric_name(metric_kind, &metric.name)?;
        validate_metric_value(
            metric_kind,
            &metric.name,
            metric.value,
            &metric.value.to_string(),
        )?;

        let sum = sums_by_name.entry(metric.name.clone()).or_default();
        *sum += metric.value;
        if !sum.is_finite() {
            return Err(BenchmarkObservationError::MetricSumNotFinite {
                metric_kind,
                metric_name: metric.name,
            });
        }
    }

    Ok(sums_by_name
        .into_iter()
        .map(|(name, value)| BenchmarkMetric { name, value })
        .collect())
}

fn metric_name_set(metrics: &[BenchmarkMetric]) -> BTreeSet<String> {
    metrics.iter().map(|metric| metric.name.clone()).collect()
}

fn parse_legacy_duration_to_ms(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let value = parse_leading_number(trimmed)?;
    let unit = trimmed
        .trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.')
        .trim();

    if unit.starts_with("ns") {
        Some(value / 1_000_000.0)
    } else if unit.starts_with("us") || unit.starts_with("µs") {
        Some(value / 1_000.0)
    } else if unit.starts_with("ms") {
        Some(value)
    } else if unit.starts_with('s') {
        Some(value * 1_000.0)
    } else {
        None
    }
}

fn parse_leading_number(text: &str) -> Option<f64> {
    let end = text
        .char_indices()
        .find_map(|(index, ch)| {
            if ch.is_ascii_digit() || ch == '.' {
                None
            } else {
                Some(index)
            }
        })
        .unwrap_or(text.len());

    if end == 0 {
        return None;
    }

    text[..end].parse().ok()
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
