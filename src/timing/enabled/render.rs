//! Terminal renderer for the structured timing summary.
//!
//! WHAT: prints a `TimingSummaryReport` through `saying::say!` with role-based
//!      colours and structured indentation.
//! WHY:  keeping rendering separate from summary construction lets tests
//!       target pure structured data, and keeps all terminal styling in one
//!       place without manual ANSI escapes.

use super::summary::{TimingEmphasis, TimingSummaryReport, TimingSummaryRow, TimingSummarySection};

/// Print one complete report.
pub(crate) fn render_timing_summary_report(report: &TimingSummaryReport) {
    saying::say!(Bold Blue report.title);

    for section in &report.sections {
        saying::say!();
        render_section(section);
    }
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
            saying::say!(Yellow label, "  ", Yellow value);
        }
        TimingEmphasis::Ordinary | TimingEmphasis::Heading => {
            saying::say!(label, "  ", Green value);
        }
        TimingEmphasis::Suffix => {
            saying::say!(label, "  ", Dark White value);
        }
    }

    for child in &row.children {
        render_row(child, label_width, depth + 1);
    }
}

fn format_value(row: &TimingSummaryRow) -> String {
    let millis = row.total.as_secs_f64() * 1000.0;
    let mut value = format!("{millis:.2}ms");

    if row.sample_count > 1 {
        value.push_str(&format!(" across {} samples", row.sample_count));
    }
    if let Some(label) = &row.max_label {
        value.push_str(&format!(" [{label}]"));
    }

    value
}
