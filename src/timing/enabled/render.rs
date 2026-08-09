//! Terminal renderer for the structured timing summary.
//!
//! WHAT: prints a `TimingSummaryReport` through `saying::say!` with role-based
//!      colours and structured indentation.
//! WHY:  keeping rendering separate from summary construction lets tests
//!       target pure structured data, and keeps all terminal styling in one
//!       place without manual ANSI escapes. Line text is built by pure
//!       helpers so tests pin the layout without capturing terminal output.

use super::summary::{
    TimingBoundarySummary, TimingReportItem, TimingSlowestModuleSummary, TimingSummaryReport,
    TimingSummaryRow, TimingSummarySection,
};
use std::time::Duration;

const MAX_SLOWEST_MODULE_IDENTITY_WIDTH: usize = 48;

/// Print one complete report.
pub(crate) fn render_timing_summary_report(report: &TimingSummaryReport) {
    // Unit tests may intentionally share the process-global collector across
    // unrelated test threads. The pure model still records those accounting
    // issues, while production debug builds retain the internal invariant
    // check for real command sessions.
    #[cfg(not(test))]
    debug_assert!(
        report.accounting_issue.is_none(),
        "timing summary accounting invariant failed: {:?}",
        report.accounting_issue
    );
    saying::say!(
        Bold Blue report.title,
        "  ",
        Yellow format_duration(report.command_total)
    );

    for item in &report.items {
        match item {
            TimingReportItem::Section(section) => {
                saying::say!();
                render_section(section);
            }
            TimingReportItem::CompilationBoundaries(boundaries) => {
                saying::say!();
                render_boundary_section(boundaries);
            }
            TimingReportItem::SlowestModule(slowest_module) => {
                saying::say!();
                render_slowest_module(slowest_module);
            }
        }
    }
}

/// Build the exact heading line text, including the total in its own field.
///
/// The renderer colours the title and duration separately; this pure helper
/// pins the layout without capturing terminal output.
#[cfg(test)]
pub(crate) fn report_title_text(report: &TimingSummaryReport) -> String {
    format!(
        "{}  {}",
        report.title,
        format_duration(report.command_total)
    )
}

fn render_section(section: &TimingSummarySection) {
    saying::say!(Blue section.title);

    let label_width = max_row_label_width(&section.rows, 0);

    for row in &section.rows {
        render_row(row, label_width, 0);
    }
}

fn render_row(row: &TimingSummaryRow, label_width: usize, depth: usize) {
    let indent = "  ".repeat(depth);
    let indented_label = format!("{indent}{}", row.label);
    let label = format!("{indented_label:<width$}", width = label_width);
    let value = format_value(row);
    saying::say!(label, "  ", Green value);

    for child in &row.children {
        render_row(child, label_width, depth + 1);
    }
}

fn render_boundary_section(boundaries: &[TimingBoundarySummary]) {
    saying::say!(Blue boundary_section_title());

    let label_width = boundaries
        .iter()
        .map(|boundary| boundary.label.len())
        .max()
        .unwrap_or(0);
    let module_width = boundaries
        .iter()
        .map(|boundary| boundary_module_word(boundary.module_count).len())
        .max()
        .unwrap_or(0);

    for boundary in boundaries {
        let label = format!("{:<width$}", boundary.label, width = label_width);
        let module_word = format!(
            "{:<width$}",
            boundary_module_word(boundary.module_count),
            width = module_width
        );
        let value = format_duration(boundary.total);
        saying::say!(Yellow label, "  ", Dark White module_word, "  ", Yellow value);
    }
}

/// The boundary section heading, which always marks accumulated work.
pub(crate) fn boundary_section_title() -> &'static str {
    "Compilation boundaries · accumulated work"
}

fn render_slowest_module(slowest_module: &TimingSlowestModuleSummary) {
    saying::say!(Blue "Slowest module");

    let identity = truncate_logical_identity(slowest_module.identity.as_ref());
    let file_word = module_file_word(slowest_module.source_file_count);
    let size = format!("{:.1}KiB", slowest_module.source_byte_count as f64 / 1024.0);
    let value = format_duration(slowest_module.total);
    saying::say!(
        identity,
        "  ",
        Green value,
        " · ",
        Dark White format!("{file_word} · {size}")
    );
}

/// Build the exact display text for one row.
#[cfg(test)]
pub(crate) fn render_row_text(row: &TimingSummaryRow, label_width: usize, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let indented_label = format!("{indent}{}", row.label);
    let label = format!("{indented_label:<width$}", width = label_width);
    format!("{label}  {}", format_value(row))
}

/// Return the width of the widest fully indented row label in a section.
fn max_row_label_width(rows: &[TimingSummaryRow], depth: usize) -> usize {
    rows.iter()
        .map(|row| {
            let own_width = depth * 2 + row.label.len();
            own_width.max(max_row_label_width(&row.children, depth + 1))
        })
        .max()
        .unwrap_or(0)
}

/// Expose the renderer's recursive width calculation to layout tests.
#[cfg(test)]
pub(crate) fn section_label_width(rows: &[TimingSummaryRow]) -> usize {
    max_row_label_width(rows, 0)
}

/// Build the exact display text for one boundary row.
#[cfg(test)]
pub(crate) fn boundary_row_text(boundary: &TimingBoundarySummary, label_width: usize) -> String {
    boundary_row_text_with_width(
        boundary,
        label_width,
        boundary_module_word(boundary.module_count).len(),
    )
}

/// Build a boundary row with an explicit shared module-count column width.
#[cfg(test)]
pub(crate) fn boundary_row_text_with_width(
    boundary: &TimingBoundarySummary,
    label_width: usize,
    module_width: usize,
) -> String {
    let module_word = boundary_module_word(boundary.module_count);
    format!(
        "{:<label_width$}  {module_word:<module_width$}  {value}",
        boundary.label,
        label_width = label_width,
        module_word = module_word,
        module_width = module_width,
        value = format_duration(boundary.total),
    )
}

fn boundary_module_word(module_count: u64) -> String {
    let word = if module_count == 1 {
        "module"
    } else {
        "modules"
    };
    format!("{module_count} {word}")
}

/// Build the exact display text for the slowest-module row.
#[cfg(test)]
pub(crate) fn slowest_module_text(slowest_module: &TimingSlowestModuleSummary) -> String {
    let file_word = module_file_word(slowest_module.source_file_count);
    format!(
        "{}  {} · {} · {:.1}KiB",
        truncate_logical_identity(slowest_module.identity.as_ref()),
        format_duration(slowest_module.total),
        file_word,
        slowest_module.source_byte_count as f64 / 1024.0,
    )
}

/// Keep the concise report bounded while preserving the logical identity's
/// unique tail, which usually contains the useful module path suffix.
fn truncate_logical_identity(identity: &str) -> String {
    if identity.chars().count() <= MAX_SLOWEST_MODULE_IDENTITY_WIDTH {
        return identity.to_owned();
    }

    let tail_length = MAX_SLOWEST_MODULE_IDENTITY_WIDTH.saturating_sub(1);
    let tail = identity
        .chars()
        .rev()
        .take(tail_length)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}

fn module_file_word(source_file_count: u64) -> String {
    let word = if source_file_count == 1 {
        "file"
    } else {
        "files"
    };
    format!("{source_file_count} {word}")
}

/// The aggregate duration value only; never sample counts or inferred labels.
fn format_value(row: &TimingSummaryRow) -> String {
    format_duration(row.total)
}

fn format_duration(duration: Duration) -> String {
    format!("{:.2}ms", duration.as_secs_f64() * 1000.0)
}
