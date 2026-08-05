//! Structured human timing summary model.
//!
//! WHAT: builds a typed, architecture-ordered report from raw timing
//!      observations so the basic report never infers meaning from dotted
//!      metric-name prefixes.
//! WHY:  presentation policy belongs in one static descriptor table; unknown
//!       raw metrics stay available to detailed and benchmark output but never
//!       appear in the basic report by accident.

use super::{BenchmarkObservationSnapshot, TimingMetricSummary};
use std::collections::BTreeMap;
use std::time::Duration;

/// How a row's value relates to wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingMeasurementKind {
    /// One contiguous wall-clock span.
    WallSpan,
    /// Sum of repeated or parallel observations.
    Accumulated,
    /// Evidence nested inside a parent row; never added to top-level totals.
    NestedEvidence,
}

/// Visual role of a row, used by the renderer for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingEmphasis {
    Heading,
    Ordinary,
    Total,
    Suffix,
}

/// One display row in a summary section.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummaryRow {
    pub(crate) label: &'static str,
    pub(crate) kind: TimingMeasurementKind,
    pub(crate) emphasis: TimingEmphasis,
    pub(crate) total: Duration,
    pub(crate) sample_count: u64,
    pub(crate) max_label: Option<String>,
    pub(crate) children: Vec<TimingSummaryRow>,
}

/// One titled group of rows.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummarySection {
    pub(crate) title: String,
    pub(crate) rows: Vec<TimingSummaryRow>,
    pub(crate) accumulated: bool,
}

/// The complete basic-mode report.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummaryReport {
    pub(crate) title: String,
    pub(crate) command_total: Duration,
    pub(crate) sections: Vec<TimingSummarySection>,
}

/// Which command produced the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingCommandKind {
    Build,
    Check,
}

/// One raw metric's basic-mode presentation policy.
struct MetricPolicy {
    metric: &'static str,
    label: &'static str,
    kind: TimingMeasurementKind,
    section: SectionId,
    emphasis: TimingEmphasis,
    /// `None` applies to every command.
    command: Option<TimingCommandKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionId {
    Pipeline,
    Frontend,
    Backend,
}

/// The single owner of basic-mode presentation policy.
///
/// Rows appear in table order, which is the architecture order. Unknown raw
/// metrics have no entry and therefore never appear in basic output.
const BASIC_METRIC_POLICY: &[MetricPolicy] = &[
    MetricPolicy {
        metric: "command.build.total",
        label: "Command total",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Total,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metric: "command.check.total",
        label: "Command total",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Total,
        command: Some(TimingCommandKind::Check),
    },
    MetricPolicy {
        metric: "build_project.bootstrap",
        label: "Bootstrap",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metric: "command.check.bootstrap",
        label: "Bootstrap",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        command: Some(TimingCommandKind::Check),
    },
    MetricPolicy {
        metric: "stage0.directory.module_inventory",
        label: "Discover and prepare graph",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "stage0.directory.module_compile_batch",
        label: "Compile packages and project",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "build_project.backend",
        label: "Backend",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metric: "output.write_total",
        label: "Write output",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metric: "frontend.file_prepare",
        label: "Prepare source files",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "frontend.header_bind",
        label: "Bind headers",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "frontend.dependency_sort",
        label: "Order declarations",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "frontend.ast",
        label: "Semantic frontend / AST",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "frontend.hir",
        label: "HIR",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "frontend.borrow",
        label: "Borrow validation",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        command: None,
    },
    MetricPolicy {
        metric: "backend.js.lower_hir",
        label: "JS lowering",
        kind: TimingMeasurementKind::NestedEvidence,
        section: SectionId::Backend,
        emphasis: TimingEmphasis::Ordinary,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metric: "backend.js.render_html_document",
        label: "HTML rendering",
        kind: TimingMeasurementKind::NestedEvidence,
        section: SectionId::Backend,
        emphasis: TimingEmphasis::Ordinary,
        command: Some(TimingCommandKind::Build),
    },
];

/// Build a deterministic basic report from one drained snapshot.
pub(crate) fn build_timing_summary(
    snapshot: &BenchmarkObservationSnapshot,
    command: TimingCommandKind,
    succeeded: bool,
) -> TimingSummaryReport {
    let aggregates = aggregate_by_metric(snapshot);
    let command_total = command_total(&aggregates, command);

    let mut sections = Vec::new();
    if let Some(section) = build_pipeline_section(&aggregates, command, command_total) {
        sections.push(section);
    }
    if let Some(section) = build_frontend_section(&aggregates) {
        sections.push(section);
    }
    if let Some(section) = build_backend_section(&aggregates) {
        sections.push(section);
    }

    let title = if succeeded {
        "Build timings".to_owned()
    } else {
        format!(
            "Build timings · failed after {}",
            format_duration(command_total)
        )
    };

    TimingSummaryReport {
        title,
        command_total,
        sections,
    }
}

/// Aggregate raw observations by metric name, preserving the slowest label.
fn aggregate_by_metric(
    snapshot: &BenchmarkObservationSnapshot,
) -> BTreeMap<&'static str, TimingMetricSummary> {
    let mut aggregates = BTreeMap::new();
    for observation in &snapshot.timings {
        aggregates
            .entry(observation.name)
            .or_insert_with(TimingMetricSummary::default)
            .record(observation.duration, observation.label.as_deref());
    }
    aggregates
}

fn command_total(
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    command: TimingCommandKind,
) -> Duration {
    let metric = match command {
        TimingCommandKind::Build => "command.build.total",
        TimingCommandKind::Check => "command.check.total",
    };
    aggregates
        .get(metric)
        .map(|summary| summary.total)
        .unwrap_or_default()
}

fn build_pipeline_section(
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    command: TimingCommandKind,
    command_total: Duration,
) -> Option<TimingSummarySection> {
    let mut rows = Vec::new();

    for policy in BASIC_METRIC_POLICY.iter().filter(|policy| {
        policy.section == SectionId::Pipeline && policy.command.is_none_or(|c| c == command)
    }) {
        push_policy_row(&mut rows, aggregates, policy);
    }

    push_other_row(&mut rows, aggregates, command_total);

    if rows.is_empty() {
        return None;
    }

    Some(TimingSummarySection {
        title: "Build pipeline".to_owned(),
        rows,
        accumulated: false,
    })
}

fn push_policy_row(
    rows: &mut Vec<TimingSummaryRow>,
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    policy: &MetricPolicy,
) {
    if let Some(summary) = aggregates.get(policy.metric)
        && !rounds_to_zero(summary.total)
    {
        rows.push(TimingSummaryRow {
            label: policy.label,
            kind: policy.kind,
            emphasis: policy.emphasis,
            total: summary.total,
            sample_count: summary.count,
            max_label: summary.max_label.clone(),
            children: Vec::new(),
        });
    }
}

/// Compute the bounded `Other` row from wall-clock children only.
fn push_other_row(
    rows: &mut Vec<TimingSummaryRow>,
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    command_total: Duration,
) {
    let mut accounted = Duration::ZERO;
    for metric in [
        "build_project.bootstrap",
        "command.check.bootstrap",
        "stage0.directory.module_inventory",
        "stage0.directory.module_compile_batch",
        "build_project.backend",
        "output.write_total",
    ] {
        if let Some(summary) = aggregates.get(metric) {
            accounted += summary.total;
        }
    }

    let other = command_total.saturating_sub(accounted);
    let other_ms = other.as_secs_f64() * 1000.0;
    let command_ms = command_total.as_secs_f64() * 1000.0;
    let significant = other_ms >= 1.0 || (command_ms > 0.0 && other_ms >= command_ms * 0.02);

    if significant && !rounds_to_zero(other) {
        rows.push(TimingSummaryRow {
            label: "Other",
            kind: TimingMeasurementKind::WallSpan,
            emphasis: TimingEmphasis::Ordinary,
            total: other,
            sample_count: 1,
            max_label: None,
            children: Vec::new(),
        });
    }
}

fn build_frontend_section(
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
) -> Option<TimingSummarySection> {
    let mut rows = Vec::new();

    for policy in BASIC_METRIC_POLICY
        .iter()
        .filter(|policy| policy.section == SectionId::Frontend)
    {
        push_policy_row(&mut rows, aggregates, policy);
    }

    if rows.is_empty() {
        return None;
    }

    let module_count = aggregates
        .get("frontend.ast")
        .map(|summary| summary.count)
        .unwrap_or_default();
    let module_word = if module_count == 1 {
        "module"
    } else {
        "modules"
    };

    Some(TimingSummarySection {
        title: format!("Frontend work · {module_count} {module_word} · accumulated"),
        rows,
        accumulated: true,
    })
}

fn build_backend_section(
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
) -> Option<TimingSummarySection> {
    let parent = aggregates.get("build_project.backend")?;
    if rounds_to_zero(parent.total) {
        return None;
    }

    let mut children = Vec::new();
    for policy in BASIC_METRIC_POLICY
        .iter()
        .filter(|policy| policy.section == SectionId::Backend)
    {
        push_significant_child(&mut children, aggregates, policy, parent.total);
    }

    if children.is_empty() {
        return None;
    }

    Some(TimingSummarySection {
        title: "Backend".to_owned(),
        rows: children,
        accumulated: false,
    })
}

/// Show a nested child only when it is at least 1ms and 5% of its parent.
fn push_significant_child(
    rows: &mut Vec<TimingSummaryRow>,
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    policy: &MetricPolicy,
    parent_total: Duration,
) {
    let Some(summary) = aggregates.get(policy.metric) else {
        return;
    };
    let child_ms = summary.total.as_secs_f64() * 1000.0;
    let parent_ms = parent_total.as_secs_f64() * 1000.0;
    let significant = child_ms >= 1.0 && (parent_ms > 0.0 && child_ms >= parent_ms * 0.05);

    if significant && !rounds_to_zero(summary.total) {
        rows.push(TimingSummaryRow {
            label: policy.label,
            kind: policy.kind,
            emphasis: policy.emphasis,
            total: summary.total,
            sample_count: summary.count,
            max_label: summary.max_label.clone(),
            children: Vec::new(),
        });
    }
}

/// Whether a duration would render as `0.00ms` after two-decimal rounding.
fn rounds_to_zero(duration: Duration) -> bool {
    duration.as_secs_f64() * 1000.0 < 0.005
}

fn format_duration(duration: Duration) -> String {
    format!("{:.2}ms", duration.as_secs_f64() * 1000.0)
}
