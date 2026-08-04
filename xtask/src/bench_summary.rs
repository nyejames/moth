//! Monthly benchmark summary writer
//!
//! This module generates and updates tracked Markdown summaries under
//! `benchmarks/summaries/YYYY-MM-Summary.md`. Each file contains:
//!
//! 1. A regenerated top section with per-system initial/latest group averages.
//! 2. An append-only list of run entries with change lines and group averages.
//!
//! WHAT: Reads local raw history, builds summary blocks, and writes/updates
//!       the public monthly Markdown file.
//! WHY:  Provides a compact, human-readable, tracked record of benchmark
//!       trends without committing raw per-case data.

use crate::bench_history::{LocalRunRecord, read_local_runs, to_case_results, to_group_stats};
use crate::bench_types::{
    BENCHMARK_PROTOCOL_VERSION, BenchmarkChangeKind, BenchmarkComparison, BenchmarkGroupStats,
    BenchmarkRun, BenchmarkThresholds, SuiteStats, calculate_stage_movement,
    format_stage_movement_line,
};
use std::fs;
use std::path::PathBuf;

const SECTION_SEPARATOR: &str = "---------------------";

/// Parsed representation of a single run entry in the summary file.
///
/// WHAT: Structured form of the "# Suite / Display (ID): Date" heading + body
/// WHY: Enables safe replacement of consecutive no-change entries and
///      keeps CLI and frontend run entries separate.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryRunEntry {
    suite_kind_label: String,
    display_name: String,
    public_system_id: String,
    timestamp_text: String,
    body: String,
    raw: String,
}

impl SummaryRunEntry {
    /// Render the entry back to Markdown.
    fn to_markdown(&self) -> String {
        format!(
            "# {} / {} ({}): {}\n{}\n",
            self.suite_kind_label,
            self.display_name,
            self.public_system_id,
            self.timestamp_text,
            self.body
        )
    }
}

/// Wrapper that preserves unparseable legacy entries.
///
/// WHAT: Either a fully parsed run entry or raw text we cannot safely interpret
/// WHY: Malformed entries must not be dropped during summary updates
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedSummaryRunEntry {
    Parsed(SummaryRunEntry),
    Raw(String),
}

impl ParsedSummaryRunEntry {
    /// Render back to Markdown, regardless of variant.
    fn to_markdown(&self) -> String {
        match self {
            ParsedSummaryRunEntry::Parsed(entry) => entry.to_markdown(),
            ParsedSummaryRunEntry::Raw(text) => text.clone(),
        }
    }
}

/// Update or create the monthly summary for the current run.
///
/// Owns the tracked-summary default-thread policy. Fixed-thread runs no-op
/// without reading or writing the summary so the public surface stays a
/// default-thread signal without mixed-identity comparisons.
///
/// - For default-thread runs: loads all local raw runs for this month.
/// - Builds the current system summary block (initial vs latest) from
///   default-thread runs only.
/// - Reads existing summary file and preserves other systems blocks.
/// - Regenerates the summary section, then appends or replaces the new run entry.
/// - Only called in Record mode.
pub fn update_monthly_summary(
    run: &BenchmarkRun,
    comparison: &BenchmarkComparison,
    paths: &crate::benchmark_run::BenchmarkPaths,
) -> Result<(), String> {
    // Defense in depth: a non-clean run must never update the tracked summary,
    // even if a caller bypasses the recording gate.
    if !run.git_revision.is_clean_committed() {
        return Err(
            "refusing to update the tracked summary from a run that is not clean and committed"
                .to_string(),
        );
    }

    // Fixed-thread runs stay local-only. The tracked summary is a default-thread
    // signal, so a fixed-thread run no-ops before any summary read or write.
    if run.thread_count.is_some() {
        return Ok(());
    }

    let month_key = run.timestamp.month_key();
    let path = summary_path(paths, &month_key);
    let suite_kind_label = run.suite_kind.display_label().to_string();
    let persisted_suite_kind = run.suite_kind.persisted_name();

    let month_runs = load_month_runs(paths, &month_key)?;

    // Only default-thread runs feed the public summary top block so a
    // recorded fixed-thread run can never become the initial or latest record.
    let system_runs: Vec<&LocalRunRecord> = month_runs
        .iter()
        .filter(|r| {
            comparable_summary_record(
                r,
                &run.system.public_system_id,
                persisted_suite_kind,
                run.thread_count,
            )
        })
        .collect();

    // Read existing summary file if present
    let (mut other_blocks, mut existing_runs) = if path.exists() {
        let content =
            fs::read_to_string(&path).map_err(|e| format!("Failed to read summary file: {}", e))?;
        parse_existing_summary(&content, &run.system.public_system_id, &suite_kind_label)
    } else {
        (Vec::new(), Vec::new())
    };

    // Generate current system block
    let current_block =
        if let (Some(initial), Some(latest)) = (system_runs.first(), system_runs.last()) {
            generate_system_block(
                &suite_kind_label,
                &run.system.display_name,
                &run.system.public_system_id,
                initial,
                latest,
                system_runs.len(),
            )
        } else {
            // No runs found for this system in the month — should not happen
            // because the current run was just appended, but handle gracefully.
            return Ok(());
        };

    // Insert current system block at the front, keep others after
    let mut all_blocks = vec![current_block];
    all_blocks.append(&mut other_blocks);

    // Generate the new run entry and append or replace in the list
    let new_run_entry = generate_run_entry(run, comparison);
    append_or_replace_run_entry(&mut existing_runs, new_run_entry, comparison);

    // Build full content
    let month_heading = run.timestamp.format_month_heading();
    let content = build_summary_content(&month_heading, &all_blocks, &existing_runs);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create summaries directory: {}", e))?;
    }

    fs::write(&path, content).map_err(|e| format!("Failed to write summary file: {}", e))
}

/// Whether a stored run may feed the tracked monthly summary selection.
///
/// Dirty and unknown-revision records stay readable local history but can
/// never become the initial or latest public summary record.
fn comparable_summary_record(
    record: &LocalRunRecord,
    public_system_id: &str,
    suite_kind: &str,
    thread_count: Option<u32>,
) -> bool {
    record.is_clean_committed()
        && record.public_system_id == public_system_id
        && record.suite_kind == suite_kind
        && record.thread_count == thread_count
        && record.benchmark_protocol_version == BENCHMARK_PROTOCOL_VERSION
}

/// Build the file path for a given month key.
fn summary_path(paths: &crate::benchmark_run::BenchmarkPaths, month_key: &str) -> PathBuf {
    paths.summaries.join(format!("{}-Summary.md", month_key))
}

/// Load all local raw runs for a specific month.
fn load_month_runs(
    paths: &crate::benchmark_run::BenchmarkPaths,
    month_key: &str,
) -> Result<Vec<LocalRunRecord>, String> {
    let runs = read_local_runs(&paths.runs_jsonl)?;
    Ok(runs
        .into_iter()
        .filter(|r| {
            r.is_clean_committed()
                && r.month_key == month_key
                && r.benchmark_protocol_version == BENCHMARK_PROTOCOL_VERSION
        })
        .collect())
}

/// Generate a system summary block for the top section.
///
/// ```markdown
/// ## End-to-end CLI / macOS M1 (B7F2A9)
/// Change since initial benchmark: -12ms avg
/// Initial: all ~80ms, Core ~120ms, Docs ~60ms
/// Latest: all ~68ms, Core ~100ms, Docs ~55ms
/// Case spread latest: ~9ms
/// ```
///
/// `run_count` is the number of runs for this system in the current month.
/// Baseline is shown only when there is a single run; later unchanged
/// months show `0ms` instead of `baseline`.
fn generate_system_block(
    suite_kind_label: &str,
    display_name: &str,
    public_system_id: &str,
    initial: &LocalRunRecord,
    latest: &LocalRunRecord,
    run_count: usize,
) -> String {
    let initial_suite = SuiteStats {
        average_ms: initial.suite_average_ms,
        case_spread_ms: initial.suite_case_spread_ms,
    };
    let latest_suite = SuiteStats {
        average_ms: latest.suite_average_ms,
        case_spread_ms: latest.suite_case_spread_ms,
    };
    let initial_groups = to_group_stats(initial);
    let latest_groups = to_group_stats(latest);

    let change_text = format_initial_to_latest_change(initial, latest, run_count);

    format!(
        "## {} / {} ({})
Change since initial benchmark: {}
Initial: {}
Latest: {}
Case spread latest: {}
",
        suite_kind_label,
        display_name,
        public_system_id,
        change_text,
        format_group_average_list(&initial_suite, &initial_groups),
        format_group_average_list(&latest_suite, &latest_groups),
        format_case_spread_ms(latest_suite.case_spread_ms),
    )
}

/// Format the top monthly change line using shared benchmark cases.
///
/// WHAT: Compares the first and latest monthly records through the same
/// per-case comparison model used by run entries.
/// WHY: Raw suite averages become incomparable when cases are added or
/// removed, while shared cases still show the real movement.
fn format_initial_to_latest_change(
    initial: &LocalRunRecord,
    latest: &LocalRunRecord,
    run_count: usize,
) -> String {
    if run_count <= 1 {
        return "baseline".to_string();
    }

    let initial_cases = to_case_results(initial);
    let latest_cases = to_case_results(latest);
    let comparison = BenchmarkComparison::new(&latest_cases, Some(&initial_cases));

    format_month_change_line(&comparison)
}

/// Render a plain, non-bold comparison line for the monthly top block.
fn format_month_change_line(comparison: &BenchmarkComparison) -> String {
    if comparison.case_set_changed || comparison.workload_changed_case_count > 0 {
        return comparison.format_run_change_line().replace("**", "");
    }

    match comparison.change_kind {
        BenchmarkChangeKind::Baseline => "baseline".to_string(),
        BenchmarkChangeKind::NoMeasurableChange => {
            format!(
                "no measurable change: avg {}; {}/{} cases",
                format_delta_line(comparison.overall_mean_delta_ms.unwrap_or(0.0)),
                comparison.compared_case_count,
                comparison.current_case_count,
            )
        }
        BenchmarkChangeKind::Faster | BenchmarkChangeKind::Slower => {
            format!(
                "{} avg; {} faster, {} slower; {}/{} cases",
                format_delta_line(comparison.overall_mean_delta_ms.unwrap_or(0.0)),
                comparison.faster_case_count,
                comparison.slower_case_count,
                comparison.compared_case_count,
                comparison.current_case_count,
            )
        }
        BenchmarkChangeKind::Mixed => {
            format!(
                "mixed: avg {}; {} faster, {} slower; {}/{} cases",
                format_delta_line(comparison.overall_mean_delta_ms.unwrap_or(0.0)),
                comparison.faster_case_count,
                comparison.slower_case_count,
                comparison.compared_case_count,
                comparison.current_case_count,
            )
        }
    }
}

/// Generate a single run entry for the append-or-replace section.
///
/// ```markdown
/// # End-to-end CLI / macOS M1 (B7F2A9): May 10th - 15:21
/// **-10ms avg**; 1 faster, 0 slower; 8/8 cases
/// Avg: all ~68ms, Core ~100ms, Docs ~55ms
/// ```
fn generate_run_entry(run: &BenchmarkRun, comparison: &BenchmarkComparison) -> SummaryRunEntry {
    let mut body_lines = vec![
        comparison.format_run_change_line(),
        format_group_average_line(&run.suite, &run.groups),
    ];

    if comparison.change_kind != BenchmarkChangeKind::Baseline {
        let movements = calculate_stage_movement(comparison);
        if let Some(stage_line) =
            format_stage_movement_line(&movements, &BenchmarkThresholds::DEFAULT)
        {
            body_lines.push(stage_line);
        }
    }

    let body = body_lines.join("\n");
    let suite_kind_label = run.suite_kind.display_label().to_string();

    SummaryRunEntry {
        suite_kind_label: suite_kind_label.clone(),
        display_name: run.system.display_name.clone(),
        public_system_id: run.system.public_system_id.clone(),
        timestamp_text: run.timestamp.format_run_header(),
        body: body.clone(),
        raw: format!(
            "# {} / {} ({}): {}\n{}\n",
            suite_kind_label,
            run.system.display_name,
            run.system.public_system_id,
            run.timestamp.format_run_header(),
            body,
        ),
    }
}

/// Round a millisecond value to the nearest whole number.
///
/// 68.4 -> 68, 68.5 -> 69
fn round_ms(value: f64) -> i64 {
    value.round() as i64
}

/// Format a delta value for public display.
///
/// -12.4 -> "-12ms", 8.5 -> "+9ms", 0.0 -> "0ms"
fn format_delta_line(value: f64) -> String {
    let rounded = round_ms(value);
    if rounded > 0 {
        format!("+{}ms", rounded)
    } else {
        format!("{}ms", rounded)
    }
}

/// Format an average millisecond value with the approximate marker.
fn format_average_ms(value: f64) -> String {
    format!("~{}ms", round_ms(value))
}

/// Format cross-case spread without implying measurement uncertainty.
fn format_case_spread_ms(value: f64) -> String {
    format_average_ms(value)
}

fn format_group_average_line(suite: &SuiteStats, groups: &[BenchmarkGroupStats]) -> String {
    format!("Avg: {}", format_group_average_list(suite, groups))
}

fn format_group_average_list(suite: &SuiteStats, groups: &[BenchmarkGroupStats]) -> String {
    let mut parts = vec![format!("all {}", format_average_ms(suite.average_ms))];

    parts.extend(groups.iter().map(|group| {
        format!(
            "{} {}",
            group_display_label(&group.group_name),
            format_average_ms(group.average_ms)
        )
    }));

    parts.join(", ")
}

/// Render one group name through the typed display label, keeping legacy
/// persisted names readable.
fn group_display_label(group_name: &str) -> &str {
    match crate::bench_types::BenchmarkGroup::parse_spelling(group_name) {
        Some(group) => group.display_label(),
        None => group_name,
    }
}

/// Parse an existing summary file.
///
/// Supports both the current dashed-line format and the legacy HTML-comment
/// format so the first rewrite of an old file preserves existing entries.
///
/// Returns:
/// - Other systems blocks (excluding the current system and suite kind).
/// - Existing run entries as parsed or raw variants.
fn parse_existing_summary(
    content: &str,
    current_public_id: &str,
    current_suite_kind: &str,
) -> (Vec<String>, Vec<ParsedSummaryRunEntry>) {
    // Try new format first: dashed separator
    if let Some(pos) = content.find(SECTION_SEPARATOR) {
        let summary_section = &content[..pos];
        let runs_section = &content[pos + SECTION_SEPARATOR.len()..];
        let runs_section = runs_section.strip_prefix('\n').unwrap_or(runs_section);

        let other_blocks =
            parse_summary_blocks(summary_section, current_public_id, current_suite_kind);
        let run_entries = parse_run_entries(runs_section);
        return (other_blocks, run_entries);
    }

    // Fall back to old format with HTML comment markers
    const OLD_SUMMARY_START: &str = "<!-- BENCHMARK_MONTH_SUMMARY_START -->";
    const OLD_SUMMARY_END: &str = "<!-- BENCHMARK_MONTH_SUMMARY_END -->";
    const OLD_RUNS_START: &str = "<!-- BENCHMARK_RUNS_START -->";

    // Extract run entries section
    let run_entries = if let Some(runs_start) = content.find(OLD_RUNS_START) {
        let after_marker = runs_start + OLD_RUNS_START.len();
        let runs_text = &content[after_marker..];
        parse_run_entries(runs_text)
    } else {
        Vec::new()
    };

    // Extract summary section between markers
    let summary_section = if let Some(start) = content.find(OLD_SUMMARY_START) {
        if let Some(end) = content.find(OLD_SUMMARY_END) {
            &content[start + OLD_SUMMARY_START.len()..end]
        } else {
            ""
        }
    } else {
        ""
    };

    let other_blocks = parse_summary_blocks(summary_section, current_public_id, current_suite_kind);

    (other_blocks, run_entries)
}

/// Extract the public system ID from a system summary block heading.
///
/// Supports both legacy `## macOS Apple Silicon (6D851D)` and
/// suite-aware `## End-to-end CLI / macOS Apple Silicon (6D851D)`.
/// Returns `Some("6D851D")` or `None` if the heading does not match.
fn public_id_from_system_heading(line: &str) -> Option<&str> {
    let line = line.strip_prefix("## ")?;
    let open = line.rfind('(')?;
    let close = line.rfind(')')?;
    if open >= close {
        return None;
    }
    Some(&line[open + 1..close])
}

/// Extract the suite kind label from a system summary block heading.
///
/// For `## End-to-end CLI / macOS M1 (B7F2A9)` returns `Some("End-to-end CLI")`.
/// For legacy `## macOS M1 (B7F2A9)` returns `None`.
fn suite_kind_from_system_heading(line: &str) -> Option<&str> {
    let line = line.strip_prefix("## ")?;
    // If there's a " / " before the last "(", the part before it is the suite kind.
    let open = line.rfind('(')?;
    let separator = line[..open].find(" / ")?;
    Some(line[..separator].trim())
}

/// Extract system summary blocks from a summary section.
///
/// Each block starts with "## ". Blocks belonging to the current system
/// and suite kind are skipped. Legacy blocks without a suite kind prefix
/// are treated as End-to-end CLI.
/// Blocks with unparseable headings are preserved as "other" blocks.
fn parse_summary_blocks(
    text: &str,
    current_public_id: &str,
    current_suite_kind: &str,
) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current_block = String::new();
    let mut in_summary = false;

    for line in text.lines() {
        if line.starts_with("## ") {
            // Save previous block if it belongs to another system or suite kind
            if !current_block.is_empty()
                && should_keep_summary_block(&current_block, current_public_id, current_suite_kind)
            {
                blocks.push(current_block.trim_end().to_string());
            }
            in_summary = true;
            current_block = line.to_string() + "\n";
        } else if in_summary && !line.trim().is_empty() {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }

    // Don't forget the last block
    if !current_block.is_empty()
        && should_keep_summary_block(&current_block, current_public_id, current_suite_kind)
    {
        blocks.push(current_block.trim_end().to_string());
    }

    blocks
}

/// Decide whether a summary block should be kept when rewriting the file.
///
/// WHAT: Blocks matching both the current public_id and current suite_kind
///       are dropped so they can be regenerated. All others are preserved.
/// WHY: Prevents stale data from accumulating while keeping unrelated
///      systems and suite kinds intact.
fn should_keep_summary_block(
    block: &str,
    current_public_id: &str,
    current_suite_kind: &str,
) -> bool {
    let heading = block.lines().next().unwrap_or("");
    let public_id = public_id_from_system_heading(heading);
    let suite_kind = suite_kind_from_system_heading(heading).unwrap_or("End-to-end CLI");

    let is_current_system = public_id.map(|id| id == current_public_id).unwrap_or(false);
    let is_current_suite = suite_kind == current_suite_kind;

    // Drop only the exact (suite, system) pair we're regenerating.
    // Preserve malformed headings and all other blocks.
    !(is_current_system && is_current_suite)
}

/// Extract individual run entries from the runs section.
///
/// Each entry starts with "# " at the beginning of a line.
/// Unparseable entries are preserved as raw strings.
fn parse_run_entries(text: &str) -> Vec<ParsedSummaryRunEntry> {
    let mut entries = Vec::new();
    let mut current = Vec::new();

    for line in text.lines() {
        if line.starts_with("# ") && !current.is_empty() {
            let raw = current.join("\n");
            entries.push(
                parse_run_entry(&raw)
                    .map(ParsedSummaryRunEntry::Parsed)
                    .unwrap_or_else(|| ParsedSummaryRunEntry::Raw(raw)),
            );
            current.clear();
        }
        if line.starts_with("# ") || !current.is_empty() {
            current.push(line.to_string());
        }
    }

    if !current.is_empty() {
        let raw = current.join("\n");
        entries.push(
            parse_run_entry(&raw)
                .map(ParsedSummaryRunEntry::Parsed)
                .unwrap_or_else(|| ParsedSummaryRunEntry::Raw(raw)),
        );
    }

    entries
}

/// Parse a single raw run entry into a structured form.
///
/// Expected shapes:
/// ```markdown
/// # Suite Kind / Display Name (PUBLIC_ID): Timestamp text
/// body line 1
/// body line 2
/// ```
///
/// Legacy entries without a suite kind prefix default to End-to-end CLI.
///
/// Returns `None` if the heading does not match the expected format.
fn parse_run_entry(raw: &str) -> Option<SummaryRunEntry> {
    let mut lines = raw.lines();
    let heading = lines.next()?;
    let heading = heading.strip_prefix("# ")?;

    // Find the final ": " separator
    let colon_pos = heading.rfind(": ")?;
    let timestamp_text = heading[colon_pos + 2..].to_string();

    // Find the last "(" and ")" before the colon
    let before_colon = &heading[..colon_pos];
    let open = before_colon.rfind('(')?;
    let close = before_colon.rfind(')')?;
    if open >= close {
        return None;
    }

    let before_paren = before_colon[..open].trim();
    let public_system_id = before_colon[open + 1..close].to_string();

    // Check for "Suite Kind / Display Name" separator
    let (suite_kind_label, display_name) = if let Some(sep_pos) = before_paren.rfind(" / ") {
        (
            before_paren[..sep_pos].trim().to_string(),
            before_paren[sep_pos + 3..].trim().to_string(),
        )
    } else {
        ("End-to-end CLI".to_string(), before_paren.to_string())
    };

    let body = lines.collect::<Vec<_>>().join("\n").trim_end().to_string();

    Some(SummaryRunEntry {
        suite_kind_label,
        display_name,
        public_system_id,
        timestamp_text,
        body,
        raw: raw.to_string(),
    })
}

/// Determine whether a parsed entry represents a no-measurable-change run.
fn is_no_measurable_change_entry(entry: &SummaryRunEntry) -> bool {
    entry.body == "no measurable change since last benchmark"
        || entry
            .body
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("no measurable change"))
}

/// Append a new run entry, or replace the latest stable no-change entry for the same system.
///
/// WHAT: If the new run is NoMeasurableChange with the same case set, and the
/// most recent entry for the same public_system_id and suite kind is also
/// NoMeasurableChange, replace it instead of appending.
/// WHY: Keeps stable day-to-day summaries tidy without hiding case-set changes
///      or accidentally collapsing across suite kinds.
fn append_or_replace_run_entry(
    existing_runs: &mut Vec<ParsedSummaryRunEntry>,
    new_entry: SummaryRunEntry,
    comparison: &BenchmarkComparison,
) {
    let is_replaceable_no_change = !comparison.case_set_changed
        && comparison.change_kind == BenchmarkChangeKind::NoMeasurableChange;

    if is_replaceable_no_change {
        let latest_same_system = existing_runs.iter().rposition(|entry| match entry {
            ParsedSummaryRunEntry::Parsed(entry) => {
                entry.public_system_id == new_entry.public_system_id
                    && entry.suite_kind_label == new_entry.suite_kind_label
            }
            ParsedSummaryRunEntry::Raw(_) => false,
        });

        if let Some(index) = latest_same_system {
            let should_replace = match &existing_runs[index] {
                ParsedSummaryRunEntry::Parsed(entry) => is_no_measurable_change_entry(entry),
                ParsedSummaryRunEntry::Raw(_) => false,
            };

            if should_replace {
                existing_runs[index] = ParsedSummaryRunEntry::Parsed(new_entry);
                return;
            }
        }
    }

    existing_runs.push(ParsedSummaryRunEntry::Parsed(new_entry));
}

/// Build the complete summary Markdown content.
fn build_summary_content(
    month_heading: &str,
    system_blocks: &[String],
    run_entries: &[ParsedSummaryRunEntry],
) -> String {
    let mut content = String::new();

    // Month heading
    content.push_str(&format!("# {} Summary\n\n", month_heading));

    // Summary section
    for block in system_blocks {
        content.push_str(block);
        content.push('\n');
    }
    content.push_str(SECTION_SEPARATOR);
    content.push_str("\n\n");

    // Runs section: entries are separated by blank lines but the file must
    // not end with a trailing blank line.
    for (index, entry) in run_entries.iter().enumerate() {
        if index > 0 {
            content.push('\n');
        }
        content.push_str(entry.to_markdown().trim_end());
        content.push('\n');
    }

    content
}

#[cfg(test)]
mod tests;
