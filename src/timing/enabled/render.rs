//! Terminal renderer for the structured timing summary.
//!
//! WHAT: prints a `TimingSummaryReport` through `saying::say!` with role-based
//!      colours and structured indentation.
//! WHY:  keeping rendering separate from summary construction lets tests
//!       target pure structured data, and keeps all terminal styling in one
//!       place without manual ANSI escapes. Line text is built by pure
//!       helpers so tests pin the layout without capturing terminal output.

use super::summary::{
    TimingBoundarySummary, TimingEmphasis, TimingReportItem, TimingSlowestModuleSummary,
    TimingSummaryReport, TimingSummaryRow, TimingSummarySection,
};
use std::time::Duration;

/// Print one complete report.
pub(crate) fn render_timing_summary_report(report: &TimingSummaryReport) {
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
pub(crate) fn report_title_text(report: &TimingSummaryReport) -> String {
    format!(
        "{}  {}",
        report.title,
        format_duration(report.command_total)
    )
}

fn render_section(section: &TimingSummarySection) {
    saying::say!(Blue section.title);

    let label_width = section
        .rows
        .iter()
        .map(|row| row.label.len())
        .max()
        .unwrap_or(0);

    for row in &section.rows {
        render_row(row, label_width, 0);
    }
}

fn render_row(row: &TimingSummaryRow, label_width: usize, depth: usize) {
    let indent = "  ".repeat(depth);
    let label = format!("{indent}{:<width$}", row.label, width = label_width);
    let value = format_value(row);

    match row.emphasis {
        TimingEmphasis::Total => {
            if let Some(suffix) = &row.suffix {
                saying::say!(Yellow label, "  ", Yellow value, " ", Dark White suffix.as_ref());
            } else {
                saying::say!(Yellow label, "  ", Yellow value);
            }
        }
        TimingEmphasis::Ordinary => {
            if let Some(suffix) = &row.suffix {
                saying::say!(label, "  ", Green value, " ", Dark White suffix.as_ref());
            } else {
                saying::say!(label, "  ", Green value);
            }
        }
    }

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

    for boundary in boundaries {
        let label = format!("{:<width$}", boundary.label, width = label_width);
        let module_word = boundary_module_word(boundary.module_count);
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

    let file_word = module_file_word(slowest_module.source_file_count);
    let size = format!("{:.1}KB", slowest_module.source_byte_count as f64 / 1024.0);
    let value = format_duration(slowest_module.total);
    saying::say!(
        slowest_module.identity.as_ref(),
        "  ",
        Green value,
        " · ",
        Dark White format!("{file_word} · {size}")
    );
}

/// Build the exact display text for one row, including any explicit suffix.
pub(crate) fn render_row_text(row: &TimingSummaryRow, label_width: usize, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let label = format!("{indent}{:<width$}", row.label, width = label_width);
    let mut text = format!("{label}  {}", format_value(row));
    if let Some(suffix) = &row.suffix {
        text.push(' ');
        text.push_str(suffix);
    }
    text
}

/// Build the exact display text for one boundary row.
pub(crate) fn boundary_row_text(boundary: &TimingBoundarySummary, label_width: usize) -> String {
    let module_word = boundary_module_word(boundary.module_count);
    format!(
        "{:<width$}  {module_word}  {value}",
        boundary.label,
        width = label_width,
        module_word = module_word,
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
pub(crate) fn slowest_module_text(slowest_module: &TimingSlowestModuleSummary) -> String {
    let file_word = module_file_word(slowest_module.source_file_count);
    format!(
        "{}  {} · {} · {:.1}KB",
        slowest_module.identity,
        format_duration(slowest_module.total),
        file_word,
        slowest_module.source_byte_count as f64 / 1024.0,
    )
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
