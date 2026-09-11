//! Test-only layout text for the timing summary renderer.
//!
//! WHAT: exposes the pure line-text helpers that pin renderer layout without capturing
//!       terminal output.
//! WHY: production rendering stays in `render.rs`; these helpers exist only so tests can
//!      assert layout against the same private width and wording functions.

use super::super::summary::{
    TimingBoundarySummary, TimingSlowestModuleSummary, TimingSummaryReport, TimingSummaryRow,
};

/// Build the exact heading line text, including the total in its own field.
///
/// The renderer colours the title and duration separately; this pure helper
/// pins the layout without capturing terminal output.
pub(crate) fn report_title_text(report: &TimingSummaryReport) -> String {
    format!(
        "{}  {}",
        report.title,
        super::format_duration(report.command_total)
    )
}

/// Build the exact display text for one row.
pub(crate) fn render_row_text(row: &TimingSummaryRow, label_width: usize, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let indented_label = format!("{indent}{}", row.label);
    let label = format!("{indented_label:<width$}", width = label_width);
    format!("{label}  {}", super::format_value(row))
}

/// Expose the renderer's recursive width calculation to layout tests.
pub(crate) fn section_label_width(rows: &[TimingSummaryRow]) -> usize {
    super::max_row_label_width(rows, 0)
}

/// Build the exact display text for one boundary row.
pub(crate) fn boundary_row_text(boundary: &TimingBoundarySummary, label_width: usize) -> String {
    boundary_row_text_with_width(
        boundary,
        label_width,
        super::boundary_module_word(boundary.module_count).len(),
    )
}

/// Build a boundary row with an explicit shared module-count column width.
pub(crate) fn boundary_row_text_with_width(
    boundary: &TimingBoundarySummary,
    label_width: usize,
    module_width: usize,
) -> String {
    let module_word = super::boundary_module_word(boundary.module_count);
    format!(
        "{:<label_width$}  {module_word:<module_width$}  {value}",
        boundary.label,
        label_width = label_width,
        module_word = module_word,
        module_width = module_width,
        value = super::format_duration(boundary.total),
    )
}

/// Build the exact display text for the slowest-module row.
pub(crate) fn slowest_module_text(slowest_module: &TimingSlowestModuleSummary) -> String {
    let file_word = super::module_file_word(slowest_module.source_file_count);
    format!(
        "{}  {} · {} · {:.1}KiB",
        super::truncate_logical_identity(slowest_module.identity.as_ref()),
        super::format_duration(slowest_module.total),
        file_word,
        slowest_module.source_byte_count as f64 / 1024.0,
    )
}
