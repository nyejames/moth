//! Hotspot extraction from parsed profile data.
//!
//! WHAT: Converts `ParsedProfileSummary` into ranked hotspot functions with
//! percentage and estimated millisecond values, applying filter-mode
//! thresholds and owner bucket mapping.
//!
//! WHY: The raw parser output contains all functions with sample counts.
//! Hotspot extraction filters, ranks, and enriches that data so agents
//! can quickly identify the most relevant functions to investigate.
//!
//! # What this module owns
//! - `extract_hotspots()` entry point
//! - `ProfileHotFunction` output type with percentages and estimates
//! - Filter-mode threshold application
//! - Moth-owned vs non-Moth function prioritization
//!
//! # What this module does NOT own
//! - Profile JSON parsing (see `parse.rs`)
//! - Owner bucket definitions (see `buckets.rs`)
//! - Artifact writing (see `artifacts.rs`)

use super::buckets::{ProfileOwnerBucketMatch, match_owner_bucket};
use super::options::ProfileFilterMode;
use super::parse::{ParsedProfileSummary, ProfileEdge};

/// Minimum self-sample percent to include a non-Moth function.
///
/// Non-Moth functions (std, alloc, rayon, etc.) must exceed this
/// threshold to appear in hotspot results. This prevents low-level
/// infrastructure from crowding out actionable Moth-owned functions
/// while still surfacing genuinely hot allocation/synchronization paths.
const NON_MOTH_MIN_SELF_PCT: f64 = 5.0;

/// Minimum sample count below which a warning is generated.
///
/// Below this threshold, profiling percentages are unreliable and may
/// represent sampling noise rather than real behavior.
const LOW_SAMPLE_COUNT_THRESHOLD: usize = 100;

/// Result of extracting hotspots from a parsed profile.
///
/// WHAT: Contains the ranked hotspot functions, warnings, and summary
/// statistics needed to write `hotspots.json`.
///
/// WHY: A named struct makes the extraction output explicit and keeps
/// the JSON writing logic separate from the filtering/ranking logic.
#[derive(Debug)]
pub(crate) struct HotspotExtractionResult {
    /// Hot functions that passed the filter thresholds, ranked by inclusive %.
    pub(crate) functions: Vec<ProfileHotFunction>,
    /// Warnings generated during extraction.
    pub(crate) warnings: Vec<String>,
    /// Total sample count from the parsed profile.
    pub(crate) total_sample_count: usize,
    /// Total sample weight from the parsed profile.
    pub(crate) total_sample_weight: f64,
    /// Observation pass wall time in milliseconds (for ms estimation).
    pub(crate) wall_time_ms: f64,
    /// Whether hot function names were symbolicated enough to be actionable.
    pub(crate) symbolication: SymbolicationHealth,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SymbolicationHealth {
    pub(crate) status: SymbolicationStatus,
    pub(crate) hot_function_count: usize,
    pub(crate) raw_address_function_count: usize,
    pub(crate) raw_address_ratio: f64,
}

impl SymbolicationHealth {
    fn from_functions(functions: &[ProfileHotFunction]) -> Self {
        let hot_function_count = functions.len();
        let raw_address_function_count = functions
            .iter()
            .filter(|function| is_raw_address_function_name(&function.name))
            .count();
        let raw_address_ratio = if hot_function_count == 0 {
            0.0
        } else {
            raw_address_function_count as f64 / hot_function_count as f64
        };
        let status = if hot_function_count == 0 {
            SymbolicationStatus::NoFunctions
        } else if raw_address_ratio >= 0.5 {
            SymbolicationStatus::AddressOnly
        } else {
            SymbolicationStatus::Healthy
        };

        Self {
            status,
            hot_function_count,
            raw_address_function_count,
            raw_address_ratio,
        }
    }

    pub(crate) fn is_failed(&self) -> bool {
        matches!(self.status, SymbolicationStatus::AddressOnly)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymbolicationStatus {
    Healthy,
    AddressOnly,
    NoFunctions,
}

impl SymbolicationStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::AddressOnly => "failed_raw_addresses",
            Self::NoFunctions => "no_functions",
        }
    }
}

/// A single hot function with percentage and estimated millisecond values.
///
/// WHAT: Enriches the parsed function data with percentages relative to
/// total sample weight, estimated milliseconds from observation wall time,
/// and the owner bucket match for source-path hints.
///
/// WHY: Percentages and estimates make the data human/agent-readable
/// without mental arithmetic over raw sample counts.
#[derive(Debug)]
pub(crate) struct ProfileHotFunction {
    /// Resolved function name from the profile.
    pub(crate) name: String,
    /// Owner bucket match with label and suggested paths.
    pub(crate) bucket: ProfileOwnerBucketMatch,
    /// Inclusive sample weight (function appears anywhere in the stack).
    pub(crate) inclusive_samples: f64,
    /// Self sample weight (function is the leaf).
    pub(crate) self_samples: f64,
    /// Inclusive percentage of total sample weight.
    pub(crate) inclusive_pct: f64,
    /// Self percentage of total sample weight.
    pub(crate) self_pct: f64,
    /// Estimated inclusive milliseconds (observation wall time * inclusive_pct / 100).
    pub(crate) estimated_inclusive_ms: f64,
    /// Estimated self milliseconds (observation wall time * self_pct / 100).
    pub(crate) estimated_self_ms: f64,
    /// Top caller edges (most-weighted callers first).
    pub(crate) top_callers: Vec<ProfileEdge>,
    /// Top callee edges (most-weighted callees first).
    pub(crate) top_callees: Vec<ProfileEdge>,
}

/// Extract hotspots from a parsed profile summary.
///
/// WHAT: Applies filter-mode thresholds, maps owner buckets, ranks functions
/// by inclusive percentage, and produces estimated millisecond values from
/// the observation pass wall time.
///
/// WHY: This is the entry point called after parsing the profile. The
/// extraction produces the data written to `hotspots.json`.
pub(crate) fn extract_hotspots(
    summary: &ParsedProfileSummary,
    filter: ProfileFilterMode,
    wall_time_ms: f64,
) -> HotspotExtractionResult {
    let mut warnings: Vec<String> = summary.warnings.clone();

    // Warn when sample count is too low for reliable percentages.
    if summary.total_sample_count < LOW_SAMPLE_COUNT_THRESHOLD {
        warnings.push(format!(
            "Sample count ({}) is below {}; percentages may be unreliable.",
            summary.total_sample_count, LOW_SAMPLE_COUNT_THRESHOLD
        ));
    }

    // Convert parsed functions into hotspot entries with percentages.
    let mut candidates: Vec<ProfileHotFunction> = summary
        .functions
        .iter()
        .map(|func| {
            let bucket = match_owner_bucket(&func.name);
            let inclusive_pct = pct(func.inclusive_samples, summary.total_sample_weight);
            let self_pct = pct(func.self_samples, summary.total_sample_weight);

            ProfileHotFunction {
                name: func.name.clone(),
                bucket,
                inclusive_samples: func.inclusive_samples,
                self_samples: func.self_samples,
                inclusive_pct,
                self_pct,
                estimated_inclusive_ms: wall_time_ms * inclusive_pct / 100.0,
                estimated_self_ms: wall_time_ms * self_pct / 100.0,
                top_callers: Vec::new(),
                top_callees: Vec::new(),
            }
        })
        .collect();

    // Apply filter-mode thresholds.
    let min_inclusive = filter.minimum_inclusive_pct();
    let min_self = filter.minimum_self_pct();
    let include_edges = filter.include_edges();

    candidates.retain(|func| {
        // Always include functions that meet the normal inclusive threshold.
        if func.inclusive_pct >= min_inclusive {
            return true;
        }

        // For Moth-owned functions, also check the normal self threshold.
        let is_moth = func.bucket.label != "unknown"
            && func.bucket.label != "other"
            && func.bucket.label != "std"
            && func.bucket.label != "core"
            && func.bucket.label != "alloc"
            && func.bucket.label != "rayon"
            && func.bucket.label != "samply/profiler";

        if is_moth && func.self_pct >= min_self {
            return true;
        }

        // For non-Moth functions, require a higher self-time threshold
        // so allocation/std/rayon hotspots are not hidden unless they are
        // genuinely significant.
        if !is_moth && func.self_pct >= NON_MOTH_MIN_SELF_PCT {
            return true;
        }

        false
    });

    // Sort by inclusive percentage descending, then self percentage descending.
    candidates.sort_by(|a, b| {
        b.inclusive_pct
            .total_cmp(&a.inclusive_pct)
            .then(b.self_pct.total_cmp(&a.self_pct))
    });

    // Apply the hot function limit.
    let limit = filter.hot_function_limit();
    candidates.truncate(limit);

    // Populate caller/callee edges if the filter mode requests them.
    if include_edges {
        let original_by_name: std::collections::HashMap<
            &str,
            &super::parse::ProfileFunctionSamples,
        > = summary
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();

        for hot in &mut candidates {
            if let Some(original) = original_by_name.get(hot.name.as_str()) {
                hot.top_callers = top_n_edges(&original.callers, 5);
                hot.top_callees = top_n_edges(&original.callees, 5);
            }
        }
    }

    let symbolication = SymbolicationHealth::from_functions(&candidates);
    if symbolication.is_failed() {
        warnings.push(format!(
            "Symbolication failed: {}/{} hot functions are raw addresses; function hotspots are not actionable.",
            symbolication.raw_address_function_count, symbolication.hot_function_count
        ));
    }

    HotspotExtractionResult {
        functions: candidates,
        warnings,
        total_sample_count: summary.total_sample_count,
        total_sample_weight: summary.total_sample_weight,
        wall_time_ms,
        symbolication,
    }
}

/// Detect raw hexadecimal address names emitted when Samply cannot resolve symbols.
pub(crate) fn is_raw_address_function_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        return trimmed[2..].chars().all(|c| c.is_ascii_hexdigit());
    }

    false
}

/// Calculate a percentage of total weight, returning 0.0 for zero total.
fn pct(value: f64, total: f64) -> f64 {
    if total > 0.0 {
        (value / total) * 100.0
    } else {
        0.0
    }
}

/// Return the top N edges from a slice, sorted by weight descending.
fn top_n_edges(edges: &[ProfileEdge], n: usize) -> Vec<ProfileEdge> {
    let mut sorted: Vec<ProfileEdge> = edges.to_vec();
    sorted.sort_by(|a, b| b.samples.total_cmp(&a.samples));
    sorted.truncate(n);
    sorted
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "hotspots_tests.rs"]
mod tests;
