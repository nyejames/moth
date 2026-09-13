//! Span-census measurement for `LocalSpan` bit-split selection.
//!
//! WHAT: walks the representative corpus, tokenizes every `.moth` and `.mtf` source, and
//!       records exact token start/length statistics plus prototype encode/decode timings
//!       for each candidate `LENGTH_BITS` split.
//! WHY:  the architecture picks the final `LocalSpan` packing from measured overflow counts
//!       and construction/resolution time. This module is the instrument. It does not choose
//!       a split and is not the production span codec.
//!
//! `.js` files appear in the same trees and are counted as excluded sources because the
//! Moth tokenizer never produces spans for them.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{self, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenizerEntryMode};
use crate::projects::html_project::style_directives::html_project_style_directives;
use std::fmt;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Candidate length-bit widths the architecture requires us to measure.
pub const CANDIDATE_LENGTH_BITS: [u32; 5] = [8, 9, 10, 11, 12];

/// Corpus roots matching the Phase 0 source-size census.
const CORPUS_ROOTS: [&str; 3] = ["benchmarks", "docs/src", "tests"];

const MOTH_EXTENSION: &str = "moth";
const MTF_EXTENSION: &str = "mtf";
const JS_EXTENSION: &str = "js";

const JS_EXCLUSION_REASON: &str = "the Moth tokenizer never produces spans for JavaScript sources";

const LONGEST_SPAN_LIMIT: usize = 8;
const EXTENDED_ENTRY_BYTES: usize = 8;

/// One completed span census over the representative corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanCensus {
    pub files_walked: usize,
    pub files_tokenized: usize,
    pub tokenize_failures: Vec<TokenizeFailure>,
    pub excluded_js: ExcludedJsSources,
    pub total_spans: usize,
    pub total_source_bytes: u64,
    pub length_distribution: LengthDistribution,
    pub start_distribution: StartDistribution,
    pub candidates: Vec<CandidateSpanStats>,
    pub longest_spans: Vec<LongSpan>,
    pub wall_time: Duration,
}

/// A corpus file the tokenizer rejected or that could not be read as UTF-8 source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizeFailure {
    pub path: String,
    pub reason: String,
}

/// `.js` files found under the corpus roots and deliberately not tokenized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedJsSources {
    pub extension: String,
    pub reason: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Span-length summary and the architecture's boundary buckets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LengthDistribution {
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

/// Observed span starts against each candidate's inline start range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartDistribution {
    pub max_start: u32,
    pub at_or_above_inline_limit: Vec<StartLimitCount>,
}

/// How many collected starts sit at or above one candidate's inline start limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartLimitCount {
    pub length_bits: u32,
    pub inline_start_limit: u32,
    pub count: usize,
}

/// Overflow and timing evidence for one `LENGTH_BITS` candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSpanStats {
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
    pub construction: Duration,
    pub resolution: Duration,
}

/// One of the longest collected token spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongSpan {
    pub path: String,
    pub length: u32,
    pub start: u32,
    pub token_kind: String,
}

/// Failure to walk the corpus, encode a candidate, or round-trip a span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanCensusError {
    pub message: String,
}

/// Histogram bucket used to classify one span length against the architecture boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthHistogramBucket {
    AtMost254,
    AtMost510,
    AtMost1022,
    AtMost2046,
    AtMost4094,
    From4095,
}

/// Prototype packing of one `(start, length)` pair for a single candidate.
///
/// This is not the production `LocalSpan` codec. It exists to time construction and
/// resolution and to prove the inline/extended counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrototypeEncodedSpan {
    Inline(NonZeroU32),
    Extended(u32),
}

struct DiscoveredCorpus {
    token_sources: Vec<PathBuf>,
    js_files: Vec<PathBuf>,
}

pub(super) struct CollectedSource {
    byte_len: u64,
    pub(super) spans: Vec<(u32, u32)>,
}

/// Tokenizer output for the whole corpus, before it is summarised.
pub(super) struct TokenizedCorpus {
    pub(super) collected: Vec<CollectedSource>,
    pub(super) failures: Vec<TokenizeFailure>,
    longest_spans: Vec<LongSpan>,
}

impl SpanCensusError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SpanCensusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for SpanCensusError {}

impl LengthHistogramBucket {
    /// Classify `length` using the architecture's closed bucket boundaries.
    pub fn for_length(length: u32) -> Self {
        if length <= 254 {
            Self::AtMost254
        } else if length <= 510 {
            Self::AtMost510
        } else if length <= 1022 {
            Self::AtMost1022
        } else if length <= 2046 {
            Self::AtMost2046
        } else if length <= 4094 {
            Self::AtMost4094
        } else {
            Self::From4095
        }
    }
}

/// Walk the corpus, tokenize every Moth source, and measure each `LocalSpan` candidate.
///
/// WHAT: fail-closed discovery, exact token byte ranges, overflow counts, prototype
///       encode/decode timings, and the longest spans that actually overflow length.
/// WHY:  bit-split selection must read measured evidence rather than the design default.
pub fn run_span_census(workspace_root: &Path) -> Result<SpanCensus, SpanCensusError> {
    let wall_start = Instant::now();
    let discovered = discover_corpus(workspace_root)?;
    let excluded_js = measure_excluded_js(workspace_root, &discovered.js_files)?;
    let tokenized = tokenize_corpus(workspace_root, &discovered.token_sources)?;
    let census = summarise_census(
        discovered.token_sources.len() + discovered.js_files.len(),
        tokenized.collected,
        tokenized.failures,
        excluded_js,
        tokenized.longest_spans,
        wall_start,
    )?;

    Ok(census)
}

/// First start byte that cannot be stored inline for `length_bits`.
pub fn inline_start_limit(length_bits: u32) -> u32 {
    1u32 << (32 - length_bits)
}

/// Largest length that still fits in the inline length code for `length_bits`.
pub fn inline_max_length(length_bits: u32) -> u32 {
    (1u32 << length_bits) - 2
}

/// Whether `(start, length)` packs inline for a candidate, using the architecture rule.
pub fn span_is_inline(length_bits: u32, start: u32, length: u32) -> bool {
    start < inline_start_limit(length_bits) && length <= inline_max_length(length_bits)
}

/// Prototype-encode `(start, length)` as inline `NonZeroU32` or an extended-table index.
///
/// Inline words store `((start << length_bits) | length) + 1` so a zero-length span at
/// byte 0 still fits `NonZeroU32`. Extended entries are append-only, matching the
/// architecture's default insertion rule.
pub fn encode_candidate_span(
    length_bits: u32,
    start: u32,
    length: u32,
    extended_table: &mut Vec<(u32, u32)>,
) -> Result<PrototypeEncodedSpan, SpanCensusError> {
    validate_length_bits(length_bits)?;

    if span_is_inline(length_bits, start, length) {
        let packed = (start << length_bits) | length;
        let stored = packed.wrapping_add(1);
        let inline = NonZeroU32::new(stored).ok_or_else(|| {
            SpanCensusError::new(
                "inline span packed to all-ones, which cannot be stored as NonZeroU32",
            )
        })?;

        return Ok(PrototypeEncodedSpan::Inline(inline));
    }

    let index = u32::try_from(extended_table.len())
        .map_err(|_| SpanCensusError::new("extended-span table exceeded u32 index space"))?;
    let max_index = inline_start_limit(length_bits).saturating_sub(2);

    if index > max_index {
        return Err(SpanCensusError::new(format!(
            "extended-span table for LENGTH_BITS={length_bits} cannot index entry {index}"
        )));
    }

    extended_table.push((start, length));
    Ok(PrototypeEncodedSpan::Extended(index))
}

/// Decode a prototype encoding back to the original `(start, length)`.
pub fn decode_candidate_span(
    length_bits: u32,
    encoded: PrototypeEncodedSpan,
    extended_table: &[(u32, u32)],
) -> Result<(u32, u32), SpanCensusError> {
    validate_length_bits(length_bits)?;

    match encoded {
        PrototypeEncodedSpan::Inline(word) => {
            let packed = word.get().wrapping_sub(1);
            let mask = (1u32 << length_bits) - 1;
            let length = packed & mask;
            let start = packed >> length_bits;
            Ok((start, length))
        }

        PrototypeEncodedSpan::Extended(index) => {
            let entry = extended_table.get(index as usize).ok_or_else(|| {
                SpanCensusError::new(format!(
                    "extended span index {index} is outside a table of {}",
                    extended_table.len()
                ))
            })?;
            Ok(*entry)
        }
    }
}

fn validate_length_bits(length_bits: u32) -> Result<(), SpanCensusError> {
    if (8..=12).contains(&length_bits) {
        Ok(())
    } else {
        Err(SpanCensusError::new(format!(
            "LENGTH_BITS={length_bits} is outside the architecture candidates 8..=12"
        )))
    }
}

fn discover_corpus(workspace_root: &Path) -> Result<DiscoveredCorpus, SpanCensusError> {
    let mut token_sources = Vec::new();
    let mut js_files = Vec::new();

    for root_name in CORPUS_ROOTS {
        let root = workspace_root.join(root_name);
        collect_corpus_files(&root, &mut token_sources, &mut js_files)?;
    }

    token_sources.sort();
    js_files.sort();

    Ok(DiscoveredCorpus {
        token_sources,
        js_files,
    })
}

fn collect_corpus_files(
    root: &Path,
    token_sources: &mut Vec<PathBuf>,
    js_files: &mut Vec<PathBuf>,
) -> Result<(), SpanCensusError> {
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            SpanCensusError::new(format!("failed to read '{}': {error}", directory.display()))
        })?;

        for entry in entries {
            let entry = entry.map_err(|error| {
                SpanCensusError::new(format!(
                    "failed to read an entry of '{}': {error}",
                    directory.display()
                ))
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                SpanCensusError::new(format!("failed to stat '{}': {error}", path.display()))
            })?;

            if metadata.is_dir() {
                pending.push(path);
                continue;
            }

            if !metadata.is_file() {
                continue;
            }

            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };

            match extension {
                MOTH_EXTENSION | MTF_EXTENSION => token_sources.push(path),
                JS_EXTENSION => js_files.push(path),
                _ => {}
            }
        }
    }

    Ok(())
}

fn measure_excluded_js(
    workspace_root: &Path,
    js_files: &[PathBuf],
) -> Result<ExcludedJsSources, SpanCensusError> {
    let mut total_bytes = 0_u64;

    for path in js_files {
        let metadata = fs::metadata(path).map_err(|error| {
            SpanCensusError::new(format!("failed to stat '{}': {error}", path.display()))
        })?;
        total_bytes += metadata.len();
        let _ = relative_portable_path(workspace_root, path)?;
    }

    Ok(ExcludedJsSources {
        extension: JS_EXTENSION.to_string(),
        reason: JS_EXCLUSION_REASON.to_string(),
        file_count: js_files.len(),
        total_bytes,
    })
}

/// The directive set the census measures with.
///
/// WHY: the docs corpus is authored against the HTML project's directives, exactly as
/// `moth_template::compile` registers them. Tokenizing with built-ins alone rejects every
/// `$html`/`$css` template, which is where the longest spans live, and a census missing its
/// longest spans selects the wrong split.
fn census_style_directives() -> Result<StyleDirectiveRegistry, SpanCensusError> {
    StyleDirectiveRegistry::merged(&html_project_style_directives())
        .map_err(|_| SpanCensusError::new("HTML project style directives failed to register"))
}

pub(super) fn tokenize_corpus(
    workspace_root: &Path,
    token_sources: &[PathBuf],
) -> Result<TokenizedCorpus, SpanCensusError> {
    let style_directives = census_style_directives()?;
    let mut collected = Vec::new();
    let mut failures = Vec::new();
    let mut longest_spans = Vec::new();

    for path in token_sources {
        let relative = relative_portable_path(workspace_root, path)?;
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(TokenizeFailure {
                    path: relative,
                    reason: format!("failed to read file: {error}"),
                });
                continue;
            }
        };
        let byte_len = bytes.len() as u64;

        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(error) => {
                failures.push(TokenizeFailure {
                    path: relative,
                    reason: format!("source is not valid UTF-8: {error}"),
                });
                continue;
            }
        };

        match tokenize_source(&relative, &source, &mut longest_spans, &style_directives) {
            Ok(spans) => collected.push(CollectedSource { byte_len, spans }),
            Err(reason) => failures.push(TokenizeFailure {
                path: relative,
                reason,
            }),
        }
    }

    Ok(TokenizedCorpus {
        collected,
        failures,
        longest_spans: sort_longest_spans(longest_spans),
    })
}

pub(super) fn tokenize_source(
    relative: &str,
    source: &str,
    longest_spans: &mut Vec<LongSpan>,
    style_directives: &StyleDirectiveRegistry,
) -> Result<Vec<(u32, u32)>, String> {
    let source_kind = source_kind_for_path(relative)?;
    let entry_mode = TokenizerEntryMode::for_source_file_kind(source_kind)
        .ok_or_else(|| format!("source kind {source_kind:?} has no tokenizer entry mode"))?;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path(relative, &mut string_table)
        .map_err(|error| format!("synthetic census path should intern: {error:?}"))?;
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        source,
        source_path,
        entry_mode,
        style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .map_err(|failure| match failure {
        lexer::TokenizeFailure::Diagnosed(diagnostic) => {
            format!("{} ({:?})", diagnostic.kind.code(), diagnostic.kind)
        }
        lexer::TokenizeFailure::Infrastructure(error) => {
            format!("tokenization infrastructure failure: {error:?}")
        }
    })?;

    let mut spans = Vec::with_capacity(file_tokens.tokens.len());

    for token in &file_tokens.tokens {
        let resolved = token.span.resolve_with(span_builder.resolver());
        let start = resolved.start();
        let end = resolved.end();

        if end < start {
            return Err(format!(
                "token {:?} has resolved end {end} before start {start}",
                token_kind_name(&token.kind)
            ));
        }

        let length = end - start;
        consider_long_span(longest_spans, relative, start, length, &token.kind);
        spans.push((start, length));
    }

    Ok(spans)
}

fn source_kind_for_path(relative: &str) -> Result<SourceFileKind, String> {
    if relative.ends_with(".moth") {
        Ok(SourceFileKind::Moth)
    } else if relative.ends_with(".mtf") {
        Ok(SourceFileKind::MothTemplate)
    } else {
        Err(format!(
            "corpus file '{relative}' is not a .moth or .mtf source"
        ))
    }
}

/// Summarise the collected corpus, timing the candidate codecs as part of the run.
///
/// `wall_start` is sampled rather than a finished `Duration`: the histograms and the five candidate
/// measurements below are a fifth of a census run, so a wall time captured before them reports the
/// tokenizer's cost and calls it the census's.
fn summarise_census(
    files_walked: usize,
    collected: Vec<CollectedSource>,
    tokenize_failures: Vec<TokenizeFailure>,
    excluded_js: ExcludedJsSources,
    longest_spans: Vec<LongSpan>,
    wall_start: Instant,
) -> Result<SpanCensus, SpanCensusError> {
    let files_tokenized = collected.len();
    let total_source_bytes = collected.iter().map(|source| source.byte_len).sum();
    let total_spans = collected.iter().map(|source| source.spans.len()).sum();
    let length_distribution = length_distribution_from(&collected);
    let start_distribution = start_distribution_from(&collected);
    let candidates = measure_candidates(&collected)?;

    Ok(SpanCensus {
        files_walked,
        files_tokenized,
        tokenize_failures,
        excluded_js,
        total_spans,
        total_source_bytes,
        length_distribution,
        start_distribution,
        candidates,
        longest_spans,
        wall_time: wall_start.elapsed(),
    })
}

fn length_distribution_from(collected: &[CollectedSource]) -> LengthDistribution {
    let mut lengths = Vec::new();
    let mut count_0_to_254 = 0;
    let mut count_255_to_510 = 0;
    let mut count_511_to_1022 = 0;
    let mut count_1023_to_2046 = 0;
    let mut count_2047_to_4094 = 0;
    let mut count_4095_and_above = 0;

    for source in collected {
        for &(_, length) in &source.spans {
            lengths.push(length);

            match LengthHistogramBucket::for_length(length) {
                LengthHistogramBucket::AtMost254 => count_0_to_254 += 1,
                LengthHistogramBucket::AtMost510 => count_255_to_510 += 1,
                LengthHistogramBucket::AtMost1022 => count_511_to_1022 += 1,
                LengthHistogramBucket::AtMost2046 => count_1023_to_2046 += 1,
                LengthHistogramBucket::AtMost4094 => count_2047_to_4094 += 1,
                LengthHistogramBucket::From4095 => count_4095_and_above += 1,
            }
        }
    }

    lengths.sort_unstable();

    LengthDistribution {
        min: lengths.first().copied().unwrap_or(0),
        median: percentile(&lengths, 50),
        p95: percentile(&lengths, 95),
        p99: percentile(&lengths, 99),
        max: lengths.last().copied().unwrap_or(0),
        count_0_to_254,
        count_255_to_510,
        count_511_to_1022,
        count_1023_to_2046,
        count_2047_to_4094,
        count_4095_and_above,
    }
}

fn start_distribution_from(collected: &[CollectedSource]) -> StartDistribution {
    let mut max_start = 0;
    let mut counts = [0_usize; CANDIDATE_LENGTH_BITS.len()];

    for source in collected {
        for &(start, _) in &source.spans {
            max_start = max_start.max(start);

            for (index, length_bits) in CANDIDATE_LENGTH_BITS.iter().enumerate() {
                if start >= inline_start_limit(*length_bits) {
                    counts[index] += 1;
                }
            }
        }
    }

    let at_or_above_inline_limit = CANDIDATE_LENGTH_BITS
        .iter()
        .zip(counts)
        .map(|(length_bits, count)| StartLimitCount {
            length_bits: *length_bits,
            inline_start_limit: inline_start_limit(*length_bits),
            count,
        })
        .collect();

    StartDistribution {
        max_start,
        at_or_above_inline_limit,
    }
}

fn sort_longest_spans(mut longest: Vec<LongSpan>) -> Vec<LongSpan> {
    longest.sort_by(|left, right| {
        right
            .length
            .cmp(&left.length)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.start.cmp(&right.start))
    });
    longest
}

fn consider_long_span(
    longest: &mut Vec<LongSpan>,
    path: &str,
    start: u32,
    length: u32,
    kind: &TokenKind,
) {
    if longest.len() >= LONGEST_SPAN_LIMIT {
        let Some(minimum_length) = longest.iter().map(|span| span.length).min() else {
            return;
        };

        if length <= minimum_length {
            return;
        }
    }

    let candidate = LongSpan {
        path: path.to_string(),
        length,
        start,
        token_kind: token_kind_name(kind),
    };

    if longest.len() < LONGEST_SPAN_LIMIT {
        longest.push(candidate);
        return;
    }

    let Some(index) = longest
        .iter()
        .enumerate()
        .min_by_key(|(_, span)| span.length)
        .map(|(index, _)| index)
    else {
        return;
    };

    longest[index] = candidate;
}

fn measure_candidates(
    collected: &[CollectedSource],
) -> Result<Vec<CandidateSpanStats>, SpanCensusError> {
    let mut candidates = Vec::with_capacity(CANDIDATE_LENGTH_BITS.len());

    for length_bits in CANDIDATE_LENGTH_BITS {
        candidates.push(measure_one_candidate(length_bits, collected)?);
    }

    Ok(candidates)
}

fn measure_one_candidate(
    length_bits: u32,
    collected: &[CollectedSource],
) -> Result<CandidateSpanStats, SpanCensusError> {
    let start_limit = inline_start_limit(length_bits);
    let max_length = inline_max_length(length_bits);
    let mut inline_spans = 0;
    let mut start_overflow_spans = 0;
    let mut length_overflow_spans = 0;
    let mut combined_extended_spans = 0;
    let mut max_extended_entries_in_one_source = 0;
    let mut encoded_sources = Vec::with_capacity(collected.len());

    let construction_start = Instant::now();

    for source in collected {
        let mut extended_table = Vec::new();
        let mut encoded = Vec::with_capacity(source.spans.len());

        for &(start, length) in &source.spans {
            if span_is_inline(length_bits, start, length) {
                inline_spans += 1;
            } else {
                combined_extended_spans += 1;

                if start >= start_limit {
                    start_overflow_spans += 1;
                }

                if length > max_length {
                    length_overflow_spans += 1;
                }
            }

            encoded.push(encode_candidate_span(
                length_bits,
                start,
                length,
                &mut extended_table,
            )?);
        }

        max_extended_entries_in_one_source =
            max_extended_entries_in_one_source.max(extended_table.len());
        encoded_sources.push((encoded, extended_table));
    }

    let construction = construction_start.elapsed();
    let resolution_start = Instant::now();

    for (source, (encoded, extended_table)) in collected.iter().zip(&encoded_sources) {
        for (&(start, length), encoded_span) in source.spans.iter().zip(encoded) {
            let decoded = decode_candidate_span(length_bits, *encoded_span, extended_table)?;

            if decoded != (start, length) {
                return Err(SpanCensusError::new(format!(
                    "LENGTH_BITS={length_bits} round-trip mismatch: encoded ({start}, {length}), decoded {decoded:?}"
                )));
            }
        }
    }

    let resolution = resolution_start.elapsed();
    let total_spans = collected.iter().map(|source| source.spans.len()).sum();

    Ok(CandidateSpanStats {
        length_bits,
        inline_start_limit: start_limit,
        inline_max_length: max_length,
        total_spans,
        inline_spans,
        start_overflow_spans,
        length_overflow_spans,
        combined_extended_spans,
        max_extended_entries_in_one_source,
        estimated_extended_table_bytes: combined_extended_spans * EXTENDED_ENTRY_BYTES,
        construction,
        resolution,
    })
}

fn percentile(sorted: &[u32], percent: u32) -> u32 {
    if sorted.is_empty() {
        return 0;
    }

    let rank = (percent as usize * (sorted.len() - 1)) / 100;
    sorted[rank]
}

fn relative_portable_path(workspace_root: &Path, path: &Path) -> Result<String, SpanCensusError> {
    let relative = path.strip_prefix(workspace_root).unwrap_or(path);
    let mut segments = Vec::new();

    for component in relative.components() {
        let segment = component.as_os_str().to_str().ok_or_else(|| {
            SpanCensusError::new(format!(
                "path '{}' has a component that is not valid UTF-8",
                relative.display()
            ))
        })?;
        segments.push(segment);
    }

    Ok(segments.join("/"))
}

fn token_kind_name(kind: &TokenKind) -> String {
    let rendered = format!("{kind:?}");

    match rendered.find(['(', '{']) {
        Some(index) => rendered[..index].to_string(),
        None => rendered,
    }
}
