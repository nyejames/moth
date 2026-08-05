//! Structured human timing summary model.
//!
//! WHAT: builds a typed, architecture-ordered report from raw timing
//!      observations so the basic report never infers meaning from dotted
//!      metric-name prefixes.
//! WHY:  presentation policy belongs in one static descriptor table; unknown
//!       raw metrics stay available to detailed and benchmark output but never
//!       appear in the basic report by accident.

use super::{BenchmarkObservationSnapshot, TimingBoundaryId, TimingMetricSummary, TimingModuleKey};
use std::borrow::Cow;
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
    Ordinary,
    Total,
}

/// One display row in a summary section.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummaryRow {
    pub(crate) label: Cow<'static, str>,
    pub(crate) kind: TimingMeasurementKind,
    pub(crate) emphasis: TimingEmphasis,
    pub(crate) total: Duration,
    /// Explicit per-row suffix, for example a boundary's module count.
    /// Never inferred from sample counts or observation labels.
    pub(crate) suffix: Option<Cow<'static, str>>,
    pub(crate) children: Vec<TimingSummaryRow>,
}

/// One titled group of rows.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummarySection {
    pub(crate) title: String,
    pub(crate) rows: Vec<TimingSummaryRow>,
}

/// One source-package or main-project boundary row.
///
/// Phase 4 registers and populates these. The model owns the shape now so
/// boundary attribution never leaks through generic row suffixes or
/// metric-name prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimingBoundarySummary {
    pub(crate) label: Cow<'static, str>,
    pub(crate) module_count: u64,
    pub(crate) total: Duration,
}

/// The single slowest-module attribution row.
///
/// Phase 4 computes module work as preparation attributed to the module plus
/// `frontend.module.semantic_total`. The model owns the shape now so
/// absolute filesystem paths cannot appear in basic output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimingSlowestModuleSummary {
    pub(crate) identity: Cow<'static, str>,
    pub(crate) source_file_count: u64,
    pub(crate) source_byte_count: u64,
    pub(crate) total: Duration,
}

/// The complete basic-mode report.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummaryReport {
    pub(crate) title: String,
    pub(crate) command_total: Duration,
    pub(crate) sections: Vec<TimingSummarySection>,
    pub(crate) compilation_boundaries: Vec<TimingBoundarySummary>,
    pub(crate) slowest_module: Option<TimingSlowestModuleSummary>,
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

    let command_word = match command {
        TimingCommandKind::Build => "Build",
        TimingCommandKind::Check => "Check",
    };
    let title = if succeeded {
        format!("{command_word} timings {}", format_duration(command_total))
    } else {
        format!(
            "{command_word} timings · failed after {}",
            format_duration(command_total)
        )
    };

    TimingSummaryReport {
        title,
        command_total,
        sections,
        compilation_boundaries: build_boundary_summaries(snapshot),
        slowest_module: build_slowest_module_summary(snapshot),
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
            .record(observation.duration);
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

/// Sum boundary inventory and compile observations per boundary.
///
/// Package inventory and package compilation are disjoint passes, so the
/// human boundary total is accumulated work, never one contiguous wall span.
fn aggregate_boundary_totals(
    snapshot: &BenchmarkObservationSnapshot,
) -> BTreeMap<TimingBoundaryId, Duration> {
    let mut totals = BTreeMap::new();
    for observation in &snapshot.timings {
        if matches!(
            observation.name,
            "build.boundary.inventory" | "build.boundary.compile"
        ) && let Some(boundary) = observation.boundary
        {
            *totals.entry(boundary).or_insert(Duration::ZERO) += observation.duration;
        }
    }
    totals
}

/// Build boundary rows in deterministic registration order.
///
/// Registration order is the graph's deterministic package order followed by
/// the main project, so the display order never depends on event insertion.
fn build_boundary_summaries(snapshot: &BenchmarkObservationSnapshot) -> Vec<TimingBoundarySummary> {
    let totals = aggregate_boundary_totals(snapshot);
    let mut rows = Vec::new();

    for record in &snapshot.boundaries {
        let Some(total) = totals.get(&record.id).copied() else {
            continue;
        };
        if rounds_to_zero(total) {
            continue;
        }
        rows.push(TimingBoundarySummary {
            label: Cow::Owned(record.display_name.clone()),
            module_count: record.module_count,
            total,
        });
    }

    rows
}

/// Build the single slowest-module row from registered module metadata.
///
/// Module work is source preparation attributed to the module plus
/// `frontend.module.semantic_total`; both aggregates are keyed by the compact
/// module key, so shuffled event insertion cannot change the winner. The
/// earliest registered module wins ties, keeping output deterministic.
fn build_slowest_module_summary(
    snapshot: &BenchmarkObservationSnapshot,
) -> Option<TimingSlowestModuleSummary> {
    let mut preparation = BTreeMap::<TimingModuleKey, Duration>::new();
    let mut semantic_total = BTreeMap::<TimingModuleKey, Duration>::new();

    for observation in &snapshot.timings {
        let Some(module) = observation.module else {
            continue;
        };
        match observation.name {
            "frontend.file_prepare" => {
                *preparation.entry(module).or_insert(Duration::ZERO) += observation.duration;
            }
            "frontend.module.semantic_total" => {
                *semantic_total.entry(module).or_insert(Duration::ZERO) += observation.duration;
            }
            _ => {}
        }
    }

    let mut slowest: Option<TimingSlowestModuleSummary> = None;
    for record in &snapshot.modules {
        let total = preparation.get(&record.key).copied().unwrap_or_default()
            + semantic_total.get(&record.key).copied().unwrap_or_default();
        if rounds_to_zero(total) {
            continue;
        }
        let is_slowest = slowest
            .as_ref()
            .is_none_or(|current: &TimingSlowestModuleSummary| total > current.total);
        if is_slowest {
            slowest = Some(TimingSlowestModuleSummary {
                identity: Cow::Owned(record.logical_identity.clone()),
                source_file_count: record.source_file_count,
                source_byte_count: record.source_byte_count,
                total,
            });
        }
    }

    slowest
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
            label: Cow::Borrowed(policy.label),
            kind: policy.kind,
            emphasis: policy.emphasis,
            total: summary.total,
            suffix: None,
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
            label: Cow::Borrowed("Other"),
            kind: TimingMeasurementKind::WallSpan,
            emphasis: TimingEmphasis::Ordinary,
            total: other,
            suffix: None,
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

    Some(TimingSummarySection {
        title: "Frontend work · accumulated".to_owned(),
        rows,
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
            label: Cow::Borrowed(policy.label),
            kind: policy.kind,
            emphasis: policy.emphasis,
            total: summary.total,
            suffix: None,
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
