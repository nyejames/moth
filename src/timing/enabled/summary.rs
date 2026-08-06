//! Structured human timing summary model.
//!
//! WHAT: builds a typed, architecture-ordered report from raw timing
//!      observations so the basic report never infers meaning from dotted
//!      metric-name prefixes.
//! WHY:  presentation policy belongs in one static descriptor table; unknown
//!       raw metrics stay available to detailed and benchmark output but never
//!       appear in the basic report by accident.

use super::{
    BenchmarkObservationSnapshot, TimingBoundaryId, TimingCommandKind, TimingContext,
    TimingMetricSummary, TimingModuleKey,
};
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Heading text without the duration, for example `Build timings` or
    /// `Build timings · failed`. The renderer prints the duration
    /// separately in the total colour role.
    pub(crate) title: String,
    pub(crate) command_total: Duration,
    /// Top-level report items in the accepted display order: pipeline,
    /// compilation boundaries, frontend, backend, slowest module.
    pub(crate) items: Vec<TimingReportItem>,
}

/// One ordered top-level report item.
///
/// The renderer walks this list verbatim, so the accepted section order is
/// owned by the model rather than by renderer field ordering.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TimingReportItem {
    Section(TimingSummarySection),
    CompilationBoundaries(Vec<TimingBoundarySummary>),
    SlowestModule(TimingSlowestModuleSummary),
}

/// One raw metric's basic-mode presentation policy.
struct MetricPolicy {
    /// One or more raw metrics summed into this row. Multiple metrics are
    /// used when a human row owns disjoint evidence, for example direct
    /// borrow-check calls or generated-function materialisation plus its
    /// sidecar borrow rechecks.
    metrics: &'static [&'static str],
    label: &'static str,
    kind: TimingMeasurementKind,
    section: SectionId,
    emphasis: TimingEmphasis,
    /// Nested evidence rows shown only when they pass the child threshold.
    children: &'static [MetricChildPolicy],
    /// Pipeline fallback shown only when the directory pipeline rows are
    /// absent, for example the single-file `Compile frontend` row.
    fallback: bool,
    /// `None` applies to every command.
    command: Option<TimingCommandKind>,
}

/// One nested child row inside a parent policy row.
struct MetricChildPolicy {
    metric: &'static str,
    label: &'static str,
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
        metrics: &["build_project.bootstrap"],
        label: "Bootstrap",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metrics: &["command.check.bootstrap"],
        label: "Bootstrap",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Check),
    },
    MetricPolicy {
        metrics: &["stage0.directory.module_inventory"],
        label: "Discover and prepare graph",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["stage0.directory.module_compile_batch"],
        label: "Compile packages and project",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["stage0.single_file.total"],
        label: "Compile frontend",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: true,
        command: None,
    },
    MetricPolicy {
        metrics: &["build_project.backend"],
        label: "Backend",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metrics: &["output.write_total"],
        label: "Write output",
        kind: TimingMeasurementKind::WallSpan,
        section: SectionId::Pipeline,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metrics: &["frontend.file_prepare"],
        label: "Prepare source files",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.header_bind"],
        label: "Bind headers",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.dependency_sort"],
        label: "Order declarations",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.ast"],
        label: "Semantic frontend / AST",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[
            MetricChildPolicy {
                metric: "ast_build_environment_ms",
                label: "Environment, types and constants",
            },
            MetricChildPolicy {
                metric: "ast_emit_nodes_ms",
                label: "Bodies and TIR construction",
            },
            MetricChildPolicy {
                metric: "ast_finalize_ms",
                label: "Template and constant finalization",
            },
        ],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.hir"],
        label: "HIR",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.public_interface"],
        label: "Public interface",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[
            MetricChildPolicy {
                metric: "frontend.public_interface.projection",
                label: "Projection",
            },
            MetricChildPolicy {
                metric: "frontend.public_interface.finalization",
                label: "Finalization",
            },
        ],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.borrow", "frontend.borrow.exact_generated"],
        label: "Borrow validation",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["frontend.generated_functions", "frontend.borrow.generated"],
        label: "Generated functions",
        kind: TimingMeasurementKind::Accumulated,
        section: SectionId::Frontend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: None,
    },
    MetricPolicy {
        metrics: &["backend.js.lower_hir", "backend.js.lower_linked_hir"],
        label: "JS lowering",
        kind: TimingMeasurementKind::NestedEvidence,
        section: SectionId::Backend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metrics: &["backend.js.render_html_document"],
        label: "HTML rendering",
        kind: TimingMeasurementKind::NestedEvidence,
        section: SectionId::Backend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metrics: &["backend.wasm.total"],
        label: "Wasm lowering",
        kind: TimingMeasurementKind::NestedEvidence,
        section: SectionId::Backend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
        command: Some(TimingCommandKind::Build),
    },
    MetricPolicy {
        metrics: &["backend.html.tracked_assets_emit"],
        label: "Tracked assets",
        kind: TimingMeasurementKind::NestedEvidence,
        section: SectionId::Backend,
        emphasis: TimingEmphasis::Ordinary,
        children: &[],
        fallback: false,
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

    let mut items = Vec::new();
    if let Some(section) = build_pipeline_section(&aggregates, command, command_total) {
        items.push(TimingReportItem::Section(section));
    }
    let boundaries = build_boundary_summaries(snapshot);
    if !boundaries.is_empty() {
        items.push(TimingReportItem::CompilationBoundaries(boundaries));
    }
    if let Some(section) = build_frontend_section(&aggregates, snapshot) {
        items.push(TimingReportItem::Section(section));
    }
    if let Some(section) = build_backend_section(&aggregates) {
        items.push(TimingReportItem::Section(section));
    }
    if let Some(slowest_module) = build_slowest_module_summary(snapshot) {
        items.push(TimingReportItem::SlowestModule(slowest_module));
    }

    let command_word = match command {
        TimingCommandKind::Build => "Build",
        TimingCommandKind::Check => "Check",
        TimingCommandKind::Dev => "Dev",
    };
    let title = if succeeded {
        format!("{command_word} timings")
    } else {
        format!("{command_word} timings · failed")
    };

    TimingSummaryReport {
        title,
        command_total,
        items,
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
        TimingCommandKind::Dev => "command.dev.build_and_write",
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
        ) && let Some(TimingContext::Boundary(boundary)) = observation.context
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
        let Some(TimingContext::Module(module)) = observation.context else {
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
    let directory_mode = aggregates.contains_key("stage0.directory.module_inventory")
        || aggregates.contains_key("stage0.directory.module_compile_batch");

    for policy in BASIC_METRIC_POLICY.iter().filter(|policy| {
        policy.section == SectionId::Pipeline
            && !policy.fallback
            && policy.command.is_none_or(|c| {
                c == command || (command == TimingCommandKind::Dev && c == TimingCommandKind::Build)
            })
    }) {
        push_policy_row(&mut rows, aggregates, &BTreeMap::new(), policy);
    }

    if !directory_mode {
        for policy in BASIC_METRIC_POLICY
            .iter()
            .filter(|policy| policy.section == SectionId::Pipeline && policy.fallback)
        {
            push_policy_row(&mut rows, aggregates, &BTreeMap::new(), policy);
        }
    }

    push_other_row(&mut rows, aggregates, command_total, directory_mode);

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
    module_ast_children: &BTreeMap<&'static str, TimingMetricSummary>,
    policy: &MetricPolicy,
) {
    let total = policy
        .metrics
        .iter()
        .filter_map(|metric| aggregates.get(metric))
        .map(|summary| summary.total)
        .sum::<Duration>();
    if rounds_to_zero(total) {
        return;
    }

    let mut children = Vec::new();
    for child in policy.children {
        let child_aggregates = if child.metric.starts_with("ast_") {
            module_ast_children
        } else {
            aggregates
        };
        if let Some(summary) = child_aggregates.get(child.metric)
            && is_significant_child(summary.total, total)
        {
            children.push(TimingSummaryRow {
                label: Cow::Borrowed(child.label),
                kind: TimingMeasurementKind::NestedEvidence,
                emphasis: TimingEmphasis::Ordinary,
                total: summary.total,
                suffix: None,
                children: Vec::new(),
            });
        }
    }

    rows.push(TimingSummaryRow {
        label: Cow::Borrowed(policy.label),
        kind: policy.kind,
        emphasis: policy.emphasis,
        total,
        suffix: None,
        children,
    });
}

/// Compute the bounded `Other` row from wall-clock children only.
fn push_other_row(
    rows: &mut Vec<TimingSummaryRow>,
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    command_total: Duration,
    directory_mode: bool,
) {
    let mut accounted = Duration::ZERO;
    for metric in [
        "build_project.bootstrap",
        "command.check.bootstrap",
        "build_project.backend",
        "output.write_total",
    ] {
        if let Some(summary) = aggregates.get(metric) {
            accounted += summary.total;
        }
    }
    if directory_mode {
        for metric in [
            "stage0.directory.module_inventory",
            "stage0.directory.module_compile_batch",
        ] {
            if let Some(summary) = aggregates.get(metric) {
                accounted += summary.total;
            }
        }
    } else if let Some(summary) = aggregates.get("stage0.single_file.total") {
        accounted += summary.total;
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

/// Aggregate the AST child metrics from module-attributed observations only.
///
/// Config parsing and generated materialisation also record the raw `ast_*`
/// metrics; those observations stay available in raw and detailed output but
/// must never appear as children of a module's `frontend.ast` row.
fn aggregate_module_ast_children(
    snapshot: &BenchmarkObservationSnapshot,
) -> BTreeMap<&'static str, TimingMetricSummary> {
    let mut aggregates = BTreeMap::new();
    for observation in &snapshot.timings {
        if matches!(
            observation.name,
            "ast_build_environment_ms" | "ast_emit_nodes_ms" | "ast_finalize_ms"
        ) && matches!(observation.context, Some(TimingContext::Module(_)))
        {
            aggregates
                .entry(observation.name)
                .or_insert_with(TimingMetricSummary::default)
                .record(observation.duration);
        }
    }
    aggregates
}

fn build_frontend_section(
    aggregates: &BTreeMap<&'static str, TimingMetricSummary>,
    snapshot: &BenchmarkObservationSnapshot,
) -> Option<TimingSummarySection> {
    let mut rows = Vec::new();
    let module_ast_children = aggregate_module_ast_children(snapshot);

    for policy in BASIC_METRIC_POLICY
        .iter()
        .filter(|policy| policy.section == SectionId::Frontend)
    {
        push_policy_row(&mut rows, aggregates, &module_ast_children, policy);
    }

    if rows.is_empty() {
        return None;
    }

    let module_count = snapshot.modules.len();
    let title = if module_count > 0 {
        format!("Frontend work · {module_count} modules · accumulated")
    } else {
        "Frontend work · accumulated".to_owned()
    };

    Some(TimingSummarySection { title, rows })
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
    let total = policy
        .metrics
        .iter()
        .filter_map(|metric| aggregates.get(metric))
        .map(|summary| summary.total)
        .sum::<Duration>();

    if is_significant_child(total, parent_total) {
        rows.push(TimingSummaryRow {
            label: Cow::Borrowed(policy.label),
            kind: policy.kind,
            emphasis: policy.emphasis,
            total,
            suffix: None,
            children: Vec::new(),
        });
    }
}

/// Whether a nested child is at least 1ms and 5% of its parent.
fn is_significant_child(child: Duration, parent: Duration) -> bool {
    let child_ms = child.as_secs_f64() * 1000.0;
    let parent_ms = parent.as_secs_f64() * 1000.0;
    child_ms >= 1.0 && (parent_ms > 0.0 && child_ms >= parent_ms * 0.05) && !rounds_to_zero(child)
}

/// Whether a duration would render as `0.00ms` after two-decimal rounding.
fn rounds_to_zero(duration: Duration) -> bool {
    duration.as_secs_f64() * 1000.0 < 0.005
}

fn format_duration(duration: Duration) -> String {
    format!("{:.2}ms", duration.as_secs_f64() * 1000.0)
}
