//! Span-census command: measure `LocalSpan` bit-split candidates over the corpus.
//!
//! WHAT: runs the compiler's span census, prints the candidate table and length
//!       histogram, and writes a machine-readable report at `target/span-census.json`.
//! WHY:  bit-split selection needs both a readable summary and a durable report whose
//!       interrupted-run state is visible.

use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::source_tree::workspace_root;
use moth::benchmarking::{
    CandidateSpanStats, ExcludedJsSources, LengthDistribution, LongSpan, SpanCensus,
    StartDistribution, TokenizeFailure, run_span_census,
};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

/// Where the census report is written, relative to the workspace root.
pub const SPAN_CENSUS_REPORT_PATH: &str = "target/span-census.json";

/// Schema version of the span census report.
pub const SPAN_CENSUS_SCHEMA_VERSION: u32 = 1;

/// Machine-readable span census report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpanCensusReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    pub corpus_roots: Vec<String>,
    pub files_walked: usize,
    pub files_tokenized: usize,
    pub tokenize_failures: Vec<TokenizeFailureReport>,
    pub excluded_js: ExcludedJsReport,
    pub total_spans: usize,
    pub total_source_bytes: u64,
    pub length_distribution: LengthDistributionReport,
    pub start_distribution: StartDistributionReport,
    pub candidates: Vec<CandidateReport>,
    pub longest_spans: Vec<LongSpanReport>,
    pub wall_time_nanos: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenizeFailureReport {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExcludedJsReport {
    pub extension: String,
    pub reason: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LengthDistributionReport {
    pub min: u32,
    pub median: u32,
    pub p95: u32,
    pub p99: u32,
    pub max: u32,
    pub count_0_to_254: usize,
    pub count_255_to_510: usize,
    pub count_511_to_1022: usize,
    pub count_1023_to_2046: usize,
    pub count_2047_to_4094: usize,
    pub count_4095_and_above: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartDistributionReport {
    pub max_start: u32,
    pub at_or_above_inline_limit: Vec<StartLimitCountReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartLimitCountReport {
    pub length_bits: u32,
    pub inline_start_limit: u32,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateReport {
    pub length_bits: u32,
    pub inline_start_limit: u32,
    pub inline_max_length: u32,
    pub total_spans: usize,
    pub inline_spans: usize,
    pub start_overflow_spans: usize,
    pub length_overflow_spans: usize,
    pub combined_extended_spans: usize,
    pub max_extended_entries_in_one_source: usize,
    pub estimated_extended_table_bytes: usize,
    pub construction_nanos: u64,
    pub resolution_nanos: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LongSpanReport {
    pub path: String,
    pub length: u32,
    pub start: u32,
    pub token_kind: String,
}

/// Run the census, write the report, and print the summary.
///
/// The report is replaced by a `completed: false` one before the walk starts. Without that, a
/// run interrupted during the walk leaves the previous successful report untouched, and a reader
/// has no way to tell that file apart from evidence this run produced.
pub fn run_span_census_command() -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let report_path = workspace_root.join(SPAN_CENSUS_REPORT_PATH);
    let run = ReportRunIdentity::started("span-census", None);

    write_span_census_report(&report_path, &started_report(run.clone()))?;

    let census = run_span_census(&workspace_root).map_err(|error| error.message)?;
    let report = finished_report(run.completed(), &census);

    write_span_census_report(&report_path, &report)?;
    print_summary(&report);

    Ok(())
}

/// The report a run writes before it has measured anything.
///
/// The counts are zero because that is what this run has measured so far; `completed: false`
/// is what tells a reader those numbers are not a result.
fn started_report(run: ReportRunIdentity) -> SpanCensusReport {
    SpanCensusReport {
        schema_version: SPAN_CENSUS_SCHEMA_VERSION,
        run,
        corpus_roots: corpus_roots(),
        files_walked: 0,
        files_tokenized: 0,
        tokenize_failures: Vec::new(),
        excluded_js: ExcludedJsReport {
            extension: "js".to_string(),
            reason: "the Moth tokenizer never produces spans for JavaScript sources".to_string(),
            file_count: 0,
            total_bytes: 0,
        },
        total_spans: 0,
        total_source_bytes: 0,
        length_distribution: LengthDistributionReport {
            min: 0,
            median: 0,
            p95: 0,
            p99: 0,
            max: 0,
            count_0_to_254: 0,
            count_255_to_510: 0,
            count_511_to_1022: 0,
            count_1023_to_2046: 0,
            count_2047_to_4094: 0,
            count_4095_and_above: 0,
        },
        start_distribution: StartDistributionReport {
            max_start: 0,
            at_or_above_inline_limit: Vec::new(),
        },
        candidates: Vec::new(),
        longest_spans: Vec::new(),
        wall_time_nanos: 0,
    }
}

fn finished_report(run: ReportRunIdentity, census: &SpanCensus) -> SpanCensusReport {
    SpanCensusReport {
        schema_version: SPAN_CENSUS_SCHEMA_VERSION,
        run,
        corpus_roots: corpus_roots(),
        files_walked: census.files_walked,
        files_tokenized: census.files_tokenized,
        tokenize_failures: census
            .tokenize_failures
            .iter()
            .map(TokenizeFailureReport::from)
            .collect(),
        excluded_js: ExcludedJsReport::from(&census.excluded_js),
        total_spans: census.total_spans,
        total_source_bytes: census.total_source_bytes,
        length_distribution: LengthDistributionReport::from(&census.length_distribution),
        start_distribution: StartDistributionReport::from(&census.start_distribution),
        candidates: census
            .candidates
            .iter()
            .map(CandidateReport::from)
            .collect(),
        longest_spans: census
            .longest_spans
            .iter()
            .map(LongSpanReport::from)
            .collect(),
        wall_time_nanos: duration_nanos(census.wall_time),
    }
}

fn corpus_roots() -> Vec<String> {
    ["benchmarks", "docs/src", "tests"]
        .iter()
        .map(|root| (*root).to_string())
        .collect()
}

fn write_span_census_report(path: &Path, report: &SpanCensusReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialise the span census report: {error}"))?;
    write_report_atomically(path, json.as_bytes())
}

fn print_summary(report: &SpanCensusReport) {
    println!(
        "span-census: {} files walked, {} tokenized, {} failed, {} spans, {} source bytes",
        report.files_walked,
        report.files_tokenized,
        report.tokenize_failures.len(),
        report.total_spans,
        report.total_source_bytes
    );
    println!(
        ".{} excluded: {} files ({} bytes) - {}",
        report.excluded_js.extension,
        report.excluded_js.file_count,
        report.excluded_js.total_bytes,
        report.excluded_js.reason
    );
    println!();
    println!("Length distribution:");
    println!(
        "  min {}  median {}  p95 {}  p99 {}  max {}",
        report.length_distribution.min,
        report.length_distribution.median,
        report.length_distribution.p95,
        report.length_distribution.p99,
        report.length_distribution.max
    );
    println!(
        "  0..=254        {}",
        report.length_distribution.count_0_to_254
    );
    println!(
        "  255..=510      {}",
        report.length_distribution.count_255_to_510
    );
    println!(
        "  511..=1022     {}",
        report.length_distribution.count_511_to_1022
    );
    println!(
        "  1023..=2046    {}",
        report.length_distribution.count_1023_to_2046
    );
    println!(
        "  2047..=4094    {}",
        report.length_distribution.count_2047_to_4094
    );
    println!(
        "  4095..         {}",
        report.length_distribution.count_4095_and_above
    );
    println!();
    println!(
        "Start distribution: max start {}",
        report.start_distribution.max_start
    );

    for limit in &report.start_distribution.at_or_above_inline_limit {
        println!(
            "  L={} at or above {}: {}",
            limit.length_bits, limit.inline_start_limit, limit.count
        );
    }

    println!();
    println!(
        " L  total     inline  start-ovf  length-ovf  extended  max/source  table-bytes  construct  resolve"
    );

    for candidate in &report.candidates {
        println!(
            "{:>2}  {:>8}  {:>8}  {:>9}  {:>10}  {:>8}  {:>10}  {:>11}  {:>9}  {:>7}",
            candidate.length_bits,
            candidate.total_spans,
            candidate.inline_spans,
            candidate.start_overflow_spans,
            candidate.length_overflow_spans,
            candidate.combined_extended_spans,
            candidate.max_extended_entries_in_one_source,
            candidate.estimated_extended_table_bytes,
            format_nanos(candidate.construction_nanos),
            format_nanos(candidate.resolution_nanos)
        );
    }

    println!();
    println!("Longest spans:");

    for span in &report.longest_spans {
        println!(
            "  {:>8} bytes  {:<20}  {} @ {}",
            span.length, span.token_kind, span.path, span.start
        );
    }

    if !report.tokenize_failures.is_empty() {
        println!();
        println!("Tokenize failures:");

        for failure in &report.tokenize_failures {
            println!("  {}: {}", failure.path, failure.reason);
        }
    }

    println!();
    println!(
        "wall time {}  report {}",
        format_nanos(report.wall_time_nanos),
        SPAN_CENSUS_REPORT_PATH
    );
}

fn duration_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn format_nanos(nanos: u64) -> String {
    if nanos >= 1_000_000_000 {
        format!("{:.3}s", nanos as f64 / 1_000_000_000.0)
    } else if nanos >= 1_000_000 {
        format!("{:.3}ms", nanos as f64 / 1_000_000.0)
    } else if nanos >= 1_000 {
        format!("{:.1}µs", nanos as f64 / 1_000.0)
    } else {
        format!("{nanos}ns")
    }
}

impl From<&TokenizeFailure> for TokenizeFailureReport {
    fn from(failure: &TokenizeFailure) -> Self {
        Self {
            path: failure.path.clone(),
            reason: failure.reason.clone(),
        }
    }
}

impl From<&ExcludedJsSources> for ExcludedJsReport {
    fn from(excluded: &ExcludedJsSources) -> Self {
        Self {
            extension: excluded.extension.clone(),
            reason: excluded.reason.clone(),
            file_count: excluded.file_count,
            total_bytes: excluded.total_bytes,
        }
    }
}

impl From<&LengthDistribution> for LengthDistributionReport {
    fn from(distribution: &LengthDistribution) -> Self {
        Self {
            min: distribution.min,
            median: distribution.median,
            p95: distribution.p95,
            p99: distribution.p99,
            max: distribution.max,
            count_0_to_254: distribution.count_0_to_254,
            count_255_to_510: distribution.count_255_to_510,
            count_511_to_1022: distribution.count_511_to_1022,
            count_1023_to_2046: distribution.count_1023_to_2046,
            count_2047_to_4094: distribution.count_2047_to_4094,
            count_4095_and_above: distribution.count_4095_and_above,
        }
    }
}

impl From<&StartDistribution> for StartDistributionReport {
    fn from(distribution: &StartDistribution) -> Self {
        Self {
            max_start: distribution.max_start,
            at_or_above_inline_limit: distribution
                .at_or_above_inline_limit
                .iter()
                .map(|limit| StartLimitCountReport {
                    length_bits: limit.length_bits,
                    inline_start_limit: limit.inline_start_limit,
                    count: limit.count,
                })
                .collect(),
        }
    }
}

impl From<&CandidateSpanStats> for CandidateReport {
    fn from(candidate: &CandidateSpanStats) -> Self {
        Self {
            length_bits: candidate.length_bits,
            inline_start_limit: candidate.inline_start_limit,
            inline_max_length: candidate.inline_max_length,
            total_spans: candidate.total_spans,
            inline_spans: candidate.inline_spans,
            start_overflow_spans: candidate.start_overflow_spans,
            length_overflow_spans: candidate.length_overflow_spans,
            combined_extended_spans: candidate.combined_extended_spans,
            max_extended_entries_in_one_source: candidate.max_extended_entries_in_one_source,
            estimated_extended_table_bytes: candidate.estimated_extended_table_bytes,
            construction_nanos: duration_nanos(candidate.construction),
            resolution_nanos: duration_nanos(candidate.resolution),
        }
    }
}

impl From<&LongSpan> for LongSpanReport {
    fn from(span: &LongSpan) -> Self {
        Self {
            path: span.path.clone(),
            length: span.length,
            start: span.start,
            token_kind: span.token_kind.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
