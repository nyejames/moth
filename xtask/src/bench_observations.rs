//! Benchmark observation parsing.
//!
//! WHAT: extracts stage timings and local performance counters from stable
//! `MOTH_BENCH` lines, while still accepting legacy human timer prose from
//! older local benchmark records.
//! WHY: raw benchmark history should preserve local diagnostic evidence without
//! making public monthly summaries noisy.

use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric};
use std::collections::{BTreeMap, HashSet};

const STAGE_PREFIXES: [(&str, &str); 10] = [
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

const STABLE_BENCH_PREFIX: &str = "MOTH_BENCH timing";
const STABLE_COUNTER_PREFIX: &str = "MOTH_BENCH counter";

pub(crate) fn parse_stdout_observations(stdout: &str) -> BenchmarkCaseObservations {
    let mut stable_timings = Vec::new();
    let mut legacy_timings = Vec::new();
    let mut counters = Vec::new();

    for raw_line in stdout.lines() {
        let line = strip_ansi_codes(raw_line);
        let trimmed = line.trim();

        if let Some(stable) = parse_stable_benchmark_line(trimmed) {
            stable_timings.push(stable);
            continue;
        }

        if let Some(counter) = parse_stable_counter_line(trimmed) {
            counters.push(counter);
            continue;
        }

        if let Some(legacy) = parse_legacy_stage_timing(trimmed) {
            legacy_timings.push(legacy);
        }
    }

    // Stable lines take precedence: skip legacy human lines for metrics that
    // already have a stable entry so new compiler output is not double-counted.
    let stable_names: HashSet<String> = stable_timings.iter().map(|m| m.name.clone()).collect();
    let mut stage_timings = stable_timings;
    for legacy in legacy_timings {
        if !stable_names.contains(&legacy.name) {
            stage_timings.push(legacy);
        }
    }

    BenchmarkCaseObservations {
        stage_timings: sum_metrics_by_name(&stage_timings),
        counters: sum_metrics_by_name(&counters),
    }
}

pub(crate) fn average_observations(
    observations: &[BenchmarkCaseObservations],
) -> BenchmarkCaseObservations {
    BenchmarkCaseObservations {
        stage_timings: average_metrics(observations.iter().map(|item| &item.stage_timings)),
        counters: average_metrics(observations.iter().map(|item| &item.counters)),
    }
}

fn parse_legacy_stage_timing(line: &str) -> Option<BenchmarkMetric> {
    for (prefix, name) in STAGE_PREFIXES {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };

        let value = parse_duration_to_ms(rest.trim())?;
        return Some(BenchmarkMetric {
            name: name.to_string(),
            value,
        });
    }

    None
}

fn parse_stable_benchmark_line(line: &str) -> Option<BenchmarkMetric> {
    let rest = line.strip_prefix(STABLE_BENCH_PREFIX)?.trim();

    // Expected shape: <name>=<value>ms
    let (name, value_with_unit) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let value = parse_duration_to_ms(value_with_unit.trim())?;

    Some(BenchmarkMetric {
        name: name.to_string(),
        value,
    })
}

fn parse_stable_counter_line(line: &str) -> Option<BenchmarkMetric> {
    let rest = line.strip_prefix(STABLE_COUNTER_PREFIX)?.trim();

    // Expected shape: <name>=<number>
    let (name, value_text) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let value: f64 = value_text.trim().parse().ok()?;
    if !value.is_finite() {
        return None;
    }

    Some(BenchmarkMetric {
        name: name.to_string(),
        value,
    })
}

fn average_metrics<'a>(
    metrics_by_iteration: impl Iterator<Item = &'a Vec<BenchmarkMetric>>,
) -> Vec<BenchmarkMetric> {
    let mut sums_by_name: BTreeMap<String, (f64, usize)> = BTreeMap::new();

    for metrics in metrics_by_iteration {
        for metric in metrics {
            let entry = sums_by_name.entry(metric.name.clone()).or_insert((0.0, 0));
            entry.0 += metric.value;
            entry.1 += 1;
        }
    }

    sums_by_name
        .into_iter()
        .map(|(name, (sum, count))| BenchmarkMetric {
            name,
            value: if count == 0 { 0.0 } else { sum / count as f64 },
        })
        .collect()
}

fn sum_metrics_by_name(metrics: &[BenchmarkMetric]) -> Vec<BenchmarkMetric> {
    let mut sums_by_name: BTreeMap<String, f64> = BTreeMap::new();

    for metric in metrics {
        *sums_by_name.entry(metric.name.clone()).or_insert(0.0) += metric.value;
    }

    sums_by_name
        .into_iter()
        .map(|(name, value)| BenchmarkMetric { name, value })
        .collect()
}

fn parse_duration_to_ms(text: &str) -> Option<f64> {
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
